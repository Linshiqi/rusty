//! The device list's pure half — `app/device.rs` asks the machine and
//! draws it: which sources are asked about the open project's part, the
//! rows each answer makes, how an answer goes into the list on screen, and
//! what the title bar's device button says about a board.
//!
//! **The list opens in the frame of the click.** It used to wait for one
//! scan of everything — the serial ports, `probe-rs list`, `wlink list`,
//! every drive letter for a UF2 bootloader, `dfu-util -l` — one after
//! another, and showed nothing until the last had answered. Now it opens on
//! what the last scan found; each source is asked on a thread of its own and
//! fills its rows in as it answers, and the foot of the list says a scan is
//! still going. Only the sources the part can use are asked
//! ([`sources_for`]).
//!
//! **A row under the pointer never moves** ([`merge`]). An answer keeps
//! every device where it was: one still there takes the answer's words in
//! place, one new goes after the last device, and one gone stays where it
//! was, marked and not to be picked, until the list is opened again. Only
//! the line at the foot changes its words.

use rusty_embed::{BootDevice, BootKind, Chip, Flasher, Probe, SerialPort, Transport};
use rusty_i18n::t;

/// Where Run goes when somebody chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chosen {
    Simulator,
    Board(Transport),
}

/// One question about what is plugged in. Each is asked on its own, so a
/// slow one holds up no other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// The serial ports: the operating system's list, at once.
    Ports,
    /// `probe-rs list`: a process.
    ProbeRs,
    /// `wlink list`: a process.
    Wlink,
    /// Every drive, for a UF2 bootloader's `INFO_UF2.TXT`.
    Uf2,
    /// `dfu-util -l`: a process.
    Dfu,
}

impl Source {
    /// The source that finds `transport` again.
    pub fn of(transport: &Transport) -> Source {
        match transport {
            Transport::Serial { .. } => Source::Ports,
            Transport::Probe {
                identifier: Some(id),
            } if id.starts_with("wlink:") => Source::Wlink,
            Transport::Probe { .. } => Source::ProbeRs,
            Transport::Uf2 { .. } => Source::Uf2,
            Transport::Dfu { .. } => Source::Dfu,
        }
    }
}

/// The sources worth asking for a part, in the order the list shows them.
/// The serial ports always: any board can be watched over one, whatever
/// writes it. A probe, a UF2 drive or a DFU device only where the part's
/// flashers say it can be written through one — an ESP32 asks no drive
/// letter and spawns no `dfu-util`. A part nobody knows asks the ports and
/// the probes, which is what the list offered every part before; no
/// unknown part is written through a bootloader (`usable_boot`).
pub fn sources_for(chip: Option<&Chip>) -> Vec<Source> {
    let Some(chip) = chip else {
        return vec![Source::Ports, Source::ProbeRs, Source::Wlink];
    };
    let has = |flasher| chip.flashers.contains(&flasher);
    let mut sources = vec![Source::Ports];
    if has(Flasher::ProbeRs) || chip.probe_rs_target.is_some() {
        sources.push(Source::ProbeRs);
    }
    if has(Flasher::Wlink) {
        sources.push(Source::Wlink);
    }
    if has(Flasher::Uf2) {
        sources.push(Source::Uf2);
    }
    if has(Flasher::Dfu) {
        sources.push(Source::Dfu);
    }
    sources
}

/// What one source answered.
#[derive(Clone, Debug)]
pub enum Answer {
    Ports(Vec<SerialPort>),
    Probes(Vec<Probe>),
    Boot(Vec<BootDevice>),
}

/// What is plugged in, as each source last answered.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub ports: Vec<SerialPort>,
    probe_rs: Vec<Probe>,
    wlink: Vec<Probe>,
    uf2: Vec<BootDevice>,
    dfu: Vec<BootDevice>,
}

impl Found {
    /// `source`'s answer, in place of its last one.
    pub fn set(&mut self, source: Source, answer: Answer) {
        match (source, answer) {
            (Source::Ports, Answer::Ports(ports)) => self.ports = ports,
            (Source::ProbeRs, Answer::Probes(probes)) => self.probe_rs = probes,
            (Source::Wlink, Answer::Probes(probes)) => self.wlink = probes,
            (Source::Uf2, Answer::Boot(boot)) => self.uf2 = boot,
            (Source::Dfu, Answer::Boot(boot)) => self.dfu = boot,
            // An answer of the wrong shape says nothing about the source.
            _ => {}
        }
    }

    /// Every probe found, probe-rs's then wlink's.
    pub fn probes(&self) -> Vec<Probe> {
        self.probe_rs.iter().chain(&self.wlink).cloned().collect()
    }

    /// Every board found waiting in a bootloader: the drives, then DFU.
    pub fn boot(&self) -> Vec<BootDevice> {
        self.uf2.iter().chain(&self.dfu).cloned().collect()
    }

    /// Whether `transport` is among what was found. A probe named by no
    /// identifier is whichever probe-rs picks, and counts as there.
    pub fn has(&self, transport: &Transport) -> bool {
        match transport {
            Transport::Serial { port } => self.ports.iter().any(|p| &p.name == port),
            Transport::Probe {
                identifier: Some(id),
            } => self.probes().iter().any(|p| &p.identifier == id),
            Transport::Probe { identifier: None } => true,
            Transport::Uf2 { drive } => self.uf2.iter().any(|b| &b.id == drive),
            Transport::Dfu { device, .. } => self.dfu.iter().any(|b| &b.id == device),
        }
    }
}

/// A row of the list: the simulator, or a device a source found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub chosen: Chosen,
    /// What found it; none for the simulator.
    pub source: Option<Source>,
    pub title: String,
    pub detail: String,
    /// Not in its source's last answer: left where it was, and not to be
    /// picked, until the list is opened again.
    pub gone: bool,
}

/// `source`'s devices as rows, in the order its answer gave them.
pub fn rows_of(source: Source, found: &Found, chip: Option<&Chip>) -> Vec<Row> {
    let row = |chosen: Transport, title: String, detail: String| Row {
        chosen: Chosen::Board(chosen),
        source: Some(source),
        title,
        detail,
        gone: false,
    };
    match source {
        Source::Ports => found
            .ports
            .iter()
            .map(|port| {
                let mut detail = vec![t!("device.serial-port")];
                if !port.boards.is_empty() {
                    detail.push(port.boards.join(", "));
                } else if let Some(bridge) = &port.bridge {
                    detail.push(bridge.clone());
                }
                if !port.likely_board {
                    detail.push(t!("device.not-a-board"));
                }
                row(
                    Transport::Serial {
                        port: port.name.clone(),
                    },
                    port.name.clone(),
                    detail.join(" · "),
                )
            })
            .collect(),
        Source::ProbeRs | Source::Wlink => {
            let probes = match source {
                Source::ProbeRs => &found.probe_rs,
                _ => &found.wlink,
            };
            probes
                .iter()
                .map(|probe| {
                    row(
                        Transport::Probe {
                            identifier: Some(probe.identifier.clone()),
                        },
                        probe.description.clone(),
                        format!("{} · {}", t!("device.debug-probe"), probe.identifier),
                    )
                })
                .collect()
        }
        Source::Uf2 | Source::Dfu => {
            let boot = match source {
                Source::Uf2 => &found.uf2,
                _ => &found.dfu,
            };
            rusty_embed::usable_boot(chip, boot)
                .iter()
                .map(|device| {
                    row(
                        rusty_embed::boot_transport(device),
                        device.label.clone(),
                        format!("{} · {}", boot_kind(device.kind), device.id),
                    )
                })
                .collect()
        }
    }
}

fn boot_kind(kind: BootKind) -> String {
    match kind {
        BootKind::Uf2 => t!("device.uf2-drive"),
        BootKind::Dfu => t!("device.dfu-device"),
    }
}

/// The list as what is known leaves it: the simulator first when it is
/// offered, then each source's devices in the order the sources are asked.
pub fn rows(simulator: bool, found: &Found, chip: Option<&Chip>, sources: &[Source]) -> Vec<Row> {
    let mut rows = Vec::new();
    if simulator {
        rows.push(Row {
            chosen: Chosen::Simulator,
            source: None,
            title: t!("desk.simulator"),
            detail: String::new(),
            gone: false,
        });
    }
    for source in sources {
        rows.extend(rows_of(*source, found, chip));
    }
    rows
}

/// `fresh`, what `source` just answered, merged into the list on screen.
/// A device still there keeps its place and takes the answer's words; a
/// new one goes after every row there is; one the answer no longer has
/// stays where it was, marked gone. Nothing already on screen moves.
pub fn merge(rows: &mut Vec<Row>, source: Source, fresh: Vec<Row>) {
    for row in rows.iter_mut().filter(|row| row.source == Some(source)) {
        row.gone = !fresh.iter().any(|new| new.chosen == row.chosen);
    }
    for new in fresh {
        match rows.iter_mut().find(|row| row.chosen == new.chosen) {
            Some(row) => {
                row.title = new.title;
                row.detail = new.detail;
                row.gone = false;
            }
            None => rows.push(new),
        }
    }
}

/// A line of the list: a row, or what the list says at its foot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Row(usize),
    /// A source is still being asked.
    Scanning,
    /// Every source answered and none found a board.
    NoneFound,
    /// Ask them all again.
    Rescan,
}

/// The rows and the foot of the list. While a scan goes on the foot says
/// so, in the place the Rescan row will take when it ends.
pub fn lines(rows: &[Row], scanning: bool) -> Vec<Line> {
    let mut lines: Vec<Line> = (0..rows.len()).map(Line::Row).collect();
    if scanning {
        lines.push(Line::Scanning);
        return lines;
    }
    let boards = rows
        .iter()
        .filter(|row| row.source.is_some() && !row.gone)
        .count();
    if boards == 0 {
        lines.push(Line::NoneFound);
    }
    lines.push(Line::Rescan);
    lines
}

/// A probe's short name: what probe-rs calls it before its USB identity
/// (`ESP JTAG -- 303a:1001:…`), or wlink's `WCH-Link#0` out of its
/// `<WCH-Link#0 nusb device> …`.
pub fn short_probe(description: &str) -> String {
    let description = description.trim();
    if let Some(rest) = description.strip_prefix('<') {
        let name = rest.split(['>', ' ']).next().unwrap_or(rest);
        return name.to_string();
    }
    description
        .split(" -- ")
        .next()
        .unwrap_or(description)
        .trim()
        .to_string()
}

/// What the device button says about a chosen board, and its tooltip: the
/// port, the probe's short name or the bootloader's, with the whole of what
/// the last scan knew of it on hover.
pub fn board_words(transport: &Transport, found: &Found) -> (String, String) {
    match transport {
        Transport::Serial { port } => {
            let mut tip = vec![port.clone(), t!("device.serial-port")];
            if let Some(known) = found.ports.iter().find(|p| &p.name == port) {
                if !known.boards.is_empty() {
                    tip.push(known.boards.join(", "));
                } else if let Some(bridge) = &known.bridge {
                    tip.push(bridge.clone());
                }
            }
            (port.clone(), tip.join(" · "))
        }
        Transport::Probe {
            identifier: Some(id),
        } => {
            let known = found.probes().into_iter().find(|p| &p.identifier == id);
            let label = known
                .as_ref()
                .map_or_else(|| id.clone(), |probe| short_probe(&probe.description));
            let tip = match &known {
                Some(probe) => format!("{} · {}", t!("device.debug-probe"), probe.description),
                None => format!("{} · {id}", t!("device.debug-probe")),
            };
            (label, tip)
        }
        Transport::Probe { identifier: None } => (t!("device.probe"), t!("device.probe")),
        Transport::Uf2 { drive } => {
            let known = found.uf2.iter().find(|b| &b.id == drive);
            let label = known.map_or_else(|| drive.clone(), |b| b.label.clone());
            (label, format!("{} · {drive}", t!("device.uf2-drive")))
        }
        Transport::Dfu { device, .. } => {
            let known = found.dfu.iter().find(|b| &b.id == device);
            let tip = match known {
                Some(b) => format!("{} · {device} · {}", t!("device.dfu-device"), b.label),
                None => format!("{} · {device}", t!("device.dfu-device")),
            };
            (format!("DFU {device}"), tip)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str) -> SerialPort {
        SerialPort {
            name: name.to_string(),
            bridge: Some("CP210x".to_string()),
            boards: Vec::new(),
            likely_board: true,
            usb: None,
        }
    }

    fn probe(id: &str, description: &str) -> Probe {
        Probe {
            identifier: id.to_string(),
            description: description.to_string(),
        }
    }

    fn found(ports: &[&str], probes: &[&str]) -> Found {
        let mut found = Found::default();
        found.set(
            Source::Ports,
            Answer::Ports(ports.iter().map(|name| port(name)).collect()),
        );
        found.set(
            Source::ProbeRs,
            Answer::Probes(probes.iter().map(|id| probe(id, id)).collect()),
        );
        found
    }

    fn chosen(rows: &[Row]) -> Vec<Chosen> {
        rows.iter().map(|row| row.chosen.clone()).collect()
    }

    fn serial(name: &str) -> Chosen {
        Chosen::Board(Transport::Serial {
            port: name.to_string(),
        })
    }

    /// A catalogued part whose flashers satisfy `wanted` — chosen by what it
    /// can do, never by name: the window names no part.
    fn part_that(wanted: impl Fn(&[Flasher]) -> bool) -> Chip {
        rusty_embed::catalog::Catalog::builtin()
            .chips()
            .iter()
            .find(|c| wanted(&c.flashers))
            .cloned()
            .expect("a catalogued part that can")
    }

    /// An answer that adds devices puts them after everything on screen,
    /// even a source listed later having answered first: no row the
    /// pointer may be over moves.
    #[test]
    fn a_new_device_goes_after_every_row_on_screen() {
        let before = found(&["COM3"], &["303a:1001"]);
        let mut rows = rows(true, &before, None, &[Source::Ports, Source::ProbeRs]);
        let shown = chosen(&rows);
        let later = found(&["COM3", "COM1"], &["303a:1001"]);
        merge(
            &mut rows,
            Source::Ports,
            rows_of(Source::Ports, &later, None),
        );
        assert_eq!(chosen(&rows)[..shown.len()], shown[..], "nothing moved");
        assert_eq!(
            rows.last().map(|row| row.chosen.clone()),
            Some(serial("COM1"))
        );
        assert_eq!(rows.len(), shown.len() + 1);
    }

    /// A device the answer no longer has keeps its place, marked, so the
    /// rows below it do not slide under the pointer; plugged back in, it is
    /// itself again in the same place.
    #[test]
    fn a_device_unplugged_stays_in_place_marked_gone() {
        let before = found(&["COM3", "COM4", "COM5"], &[]);
        let mut rows = rows(false, &before, None, &[Source::Ports]);
        merge(
            &mut rows,
            Source::Ports,
            rows_of(Source::Ports, &found(&["COM3", "COM5"], &[]), None),
        );
        assert_eq!(
            chosen(&rows),
            [serial("COM3"), serial("COM4"), serial("COM5")]
        );
        assert_eq!(
            rows.iter().map(|row| row.gone).collect::<Vec<_>>(),
            [false, true, false]
        );
        merge(
            &mut rows,
            Source::Ports,
            rows_of(Source::Ports, &before, None),
        );
        assert!(rows.iter().all(|row| !row.gone));
        assert_eq!(rows.len(), 3, "back in its own place, not added again");
    }

    /// One source's answer says nothing about another's devices.
    #[test]
    fn an_answer_touches_only_its_own_source() {
        let mut rows = rows(
            false,
            &found(&["COM3"], &["probe"]),
            None,
            &[Source::Ports, Source::ProbeRs],
        );
        merge(&mut rows, Source::Ports, Vec::new());
        assert_eq!(
            rows.iter().map(|row| row.gone).collect::<Vec<_>>(),
            [true, false],
            "the port went; the probe was not asked"
        );
    }

    /// The foot of the list: scanning in the place the Rescan row takes,
    /// and none found only once every source has answered.
    #[test]
    fn the_foot_says_scanning_until_every_source_answered() {
        let simulator = rows(true, &Found::default(), None, &[]);
        assert_eq!(lines(&simulator, true), [Line::Row(0), Line::Scanning]);
        assert_eq!(
            lines(&simulator, false),
            [Line::Row(0), Line::NoneFound, Line::Rescan],
            "the simulator is not a board"
        );
        let board = rows(false, &found(&["COM3"], &[]), None, &[Source::Ports]);
        assert_eq!(lines(&board, false), [Line::Row(0), Line::Rescan]);
        let mut gone = board.clone();
        merge(&mut gone, Source::Ports, Vec::new());
        assert_eq!(
            lines(&gone, false),
            [Line::Row(0), Line::NoneFound, Line::Rescan],
            "a board that went is not found"
        );
    }

    /// Asked only about what the part can be reached through: a part written
    /// over serial and a probe spawns no wlink and reads no drive letter; a
    /// part with a UF2 boot ROM asks the drives and not DFU; a DFU part asks
    /// dfu-util; a wlink part asks wlink; a part nobody knows asks the ports
    /// and the probes.
    #[test]
    fn a_part_is_asked_about_its_own_ways_in() {
        let serial = part_that(|f| f == [Flasher::Espflash, Flasher::ProbeRs]);
        assert_eq!(sources_for(Some(&serial)), [Source::Ports, Source::ProbeRs]);
        let uf2 = part_that(|f| f.contains(&Flasher::Uf2) && !f.contains(&Flasher::Dfu));
        assert!(sources_for(Some(&uf2)).contains(&Source::Uf2));
        assert!(!sources_for(Some(&uf2)).contains(&Source::Dfu));
        let dfu = part_that(|f| f.contains(&Flasher::Dfu));
        assert!(sources_for(Some(&dfu)).contains(&Source::Dfu));
        let wlink = part_that(|f| f.contains(&Flasher::Wlink));
        assert!(sources_for(Some(&wlink)).contains(&Source::Wlink));
        assert_eq!(
            sources_for(None),
            [Source::Ports, Source::ProbeRs, Source::Wlink]
        );
        for chip in rusty_embed::catalog::Catalog::builtin().chips() {
            assert_eq!(
                sources_for(Some(chip)).first(),
                Some(&Source::Ports),
                "{}: any board can be watched over a port",
                chip.id
            );
        }
    }

    /// A transport is found again by the source that lists it.
    #[test]
    fn a_transport_is_found_by_its_own_source() {
        let wlink = Transport::Probe {
            identifier: Some("wlink:0".to_string()),
        };
        assert_eq!(Source::of(&wlink), Source::Wlink);
        let probe = Transport::Probe {
            identifier: Some("303a:1001".to_string()),
        };
        assert_eq!(Source::of(&probe), Source::ProbeRs);
        let known = found(&["COM3"], &["303a:1001"]);
        assert!(known.has(&probe));
        assert!(known.has(&Transport::Serial {
            port: "COM3".to_string()
        }));
        assert!(!known.has(&Transport::Serial {
            port: "COM9".to_string()
        }));
    }

    /// The button names a probe by its name, not its USB identity.
    #[test]
    fn a_probe_is_named_short() {
        assert_eq!(
            short_probe("USB JTAG -- 303a:1001:34:85:18:0A:47:F4 (EspJtag)"),
            "USB JTAG"
        );
        assert_eq!(
            short_probe("<Link#0 nusb device> ID 1a86:8010 Serial 0001A0000000 (RV mode)"),
            "Link#0",
            "wlink's own form"
        );
        assert_eq!(short_probe("A probe"), "A probe");
    }

    /// What the button says is the device's own name: the port, the probe,
    /// the bootloader's board — and the tooltip says more of it.
    #[test]
    fn the_button_names_the_board_chosen() {
        let mut known = found(&["COM5"], &[]);
        known.set(
            Source::ProbeRs,
            Answer::Probes(vec![probe(
                "303a:1001:34:85",
                "USB JTAG -- 303a:1001:34:85 (EspJtag)",
            )]),
        );
        known.set(
            Source::Uf2,
            Answer::Boot(vec![BootDevice {
                kind: BootKind::Uf2,
                id: "F:\\".to_string(),
                serial: None,
                label: "BOOT-DRIVE".to_string(),
            }]),
        );
        let (label, tip) = board_words(
            &Transport::Serial {
                port: "COM5".to_string(),
            },
            &known,
        );
        assert_eq!(label, "COM5");
        assert!(tip.contains("CP210x"), "{tip}");
        let (label, tip) = board_words(
            &Transport::Probe {
                identifier: Some("303a:1001:34:85".to_string()),
            },
            &known,
        );
        assert_eq!(label, "USB JTAG");
        assert!(tip.contains("303a:1001:34:85"), "{tip}");
        let (label, _) = board_words(
            &Transport::Uf2 {
                drive: "F:\\".to_string(),
            },
            &known,
        );
        assert_eq!(label, "BOOT-DRIVE");
        // A board the last scan did not see is still named by what was chosen.
        let (label, _) = board_words(
            &Transport::Serial {
                port: "COM9".to_string(),
            },
            &known,
        );
        assert_eq!(label, "COM9");
    }
}
