//! The Registers tab: the chip's peripherals from its vendor's SVD, and the
//! picked one's registers read at every stop of a debug session — each a
//! word, and the fields in it.
//!
//! The SVD is found in the project's `.rusty/svd/` or the data directory
//! and fetched on demand, never bundled; a chip with none says which file
//! it wants and offers to fetch it, because guessing register addresses
//! would be the worst possible answer. A write-only register is never read:
//! reading one can wedge the peripheral.

use std::rc::Rc;

use rusty_embed::RegisterMap;
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{RegField, RegRow, Shell, work};

/// What the tab holds.
#[derive(Default)]
pub(super) struct Registers {
    /// `None` until read; `Some(None)` when this machine has no SVD.
    map: Option<Option<RegisterMap>>,
    picked: Option<String>,
    /// The registers whose fields are shown.
    open: std::collections::BTreeSet<String>,
    /// What was last read: the stop and the peripheral.
    read_for: Option<(String, String)>,
}

impl super::State {
    /// The SVD has not been looked for yet.
    pub(super) fn registers_unread(&self) -> bool {
        self.registers.map.is_none()
    }
}

impl App {
    /// The target runs again: the next stop reads afresh.
    pub(super) fn registers_running(&self) {
        self.state.borrow_mut().registers.read_for = None;
    }

    /// Read the chip's SVD, if this machine has one.
    pub(super) fn load_registers(self: &Rc<Self>) {
        let (chip, root) = {
            let state = self.state.borrow();
            let Some(project) = state.project.as_ref() else {
                return;
            };
            let chip = project
                .embedded
                .as_ref()
                .ok()
                .and_then(|embedded| embedded.chip.clone());
            (chip, project.root.clone())
        };
        let Some(chip) = chip else {
            self.state.borrow_mut().registers.map = Some(None);
            self.sync_registers();
            return;
        };
        work::spawn(
            move || {
                let firmware = rusty_embed::project::firmware_root(&root);
                let path = rusty_embed::svd::find(&chip, Some(&firmware))?;
                let xml = std::fs::read_to_string(path).ok()?;
                Some(rusty_embed::svd::parse(&xml))
            },
            |app, map| {
                app.state.borrow_mut().registers.map = Some(map);
                app.sync_registers();
                app.read_peripheral();
            },
        );
    }

    /// Fetch the chip's SVD from where its vendor publishes it, saying
    /// each step in the dock, then read it.
    pub(super) fn fetch_svd(self: &Rc<Self>) {
        let chip = self
            .state
            .borrow()
            .project
            .as_ref()
            .and_then(|p| p.embedded.as_ref().ok().and_then(|e| e.chip.clone()));
        let Some(chip) = chip else { return };
        self.show_panel("bottom", "output");
        work::spawn(
            move || {
                rusty_embed::svd::fetch(&chip, |line| {
                    work::deliver(line, |app, line| {
                        app.runner.say(&line, crate::run::kind::PLAIN);
                    });
                })
                .map(|_| ())
                .map_err(|e| e.to_string())
            },
            |app, fetched| match fetched {
                Ok(()) => {
                    app.state.borrow_mut().registers.map = None;
                    app.load_registers();
                    app.show_panel("bottom", "registers");
                }
                Err(reason) => app.runner.say(&reason, crate::run::kind::ERROR),
            },
        );
    }

    pub(super) fn registers_pick(self: &Rc<Self>, name: &str) {
        self.state.borrow_mut().registers.picked = Some(name.to_string());
        self.read_peripheral();
        self.sync_registers();
    }

    pub(super) fn registers_toggle(self: &Rc<Self>, name: &str) {
        {
            let mut state = self.state.borrow_mut();
            let open = &mut state.registers.open;
            if !open.remove(name) {
                open.insert(name.to_string());
            }
        }
        self.sync_registers();
    }

    /// At a stop, the picked peripheral's block is read in one request:
    /// thirty round trips per stop is what makes a register view feel
    /// broken.
    pub(super) fn read_peripheral(self: &Rc<Self>) {
        let (session_span, key) = {
            let state = self.state.borrow();
            let Some(debug) = state.debug.state.as_ref().filter(|d| d.stopped()) else {
                return;
            };
            let address = debug
                .stack
                .first()
                .map(|f| f.address.clone())
                .unwrap_or_default();
            let Some(Some(map)) = state.registers.map.as_ref() else {
                return;
            };
            let name = state
                .registers
                .picked
                .clone()
                .or_else(|| map.peripherals.first().map(|p| p.name.clone()));
            let Some(peripheral) = name
                .as_ref()
                .and_then(|name| map.peripherals.iter().find(|p| &p.name == name))
            else {
                return;
            };
            let end = peripheral
                .registers
                .iter()
                .filter(|r| r.readable)
                .map(|r| r.offset + 4)
                .max()
                .unwrap_or(4);
            (
                (peripheral.base, end.min(4096)),
                (address, peripheral.name.clone()),
            )
        };
        if self.state.borrow().registers.read_for.as_ref() == Some(&key) {
            return;
        }
        self.state.borrow_mut().registers.read_for = Some(key);
        self.debug_read(session_span.0, session_span.1);
    }

    pub(super) fn sync_registers(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let registers = &state.registers;
        shell.set_registers_loaded(registers.map.is_some());
        let Some(Some(map)) = registers.map.as_ref() else {
            shell.set_registers_svd(false);
            shell.set_registers_names(ModelRc::default());
            shell.set_registers_rows(ModelRc::default());
            return;
        };
        shell.set_registers_svd(true);
        let picked = registers
            .picked
            .clone()
            .or_else(|| map.peripherals.first().map(|p| p.name.clone()));
        let names: Vec<SharedString> = map
            .peripherals
            .iter()
            .map(|p| p.name.as_str().into())
            .collect();
        shell.set_registers_names(ModelRc::new(VecModel::from(names)));
        shell.set_registers_picked(picked.clone().unwrap_or_default().into());
        shell.set_registers_dropped(if map.dropped > 0 {
            t!("dock.registers.inherited", count = map.dropped.to_string()).into()
        } else {
            SharedString::new()
        });
        let Some(peripheral) = picked
            .as_ref()
            .and_then(|name| map.peripherals.iter().find(|p| &p.name == name))
        else {
            shell.set_registers_rows(ModelRc::default());
            return;
        };
        shell.set_registers_heading(
            format!(
                "{}  0x{:08X}  {}",
                peripheral.name, peripheral.base, peripheral.description
            )
            .into(),
        );
        let memory = state
            .debug
            .state
            .as_ref()
            .map(|d| d.memory.clone())
            .unwrap_or_default();
        let read = |address: u64| -> Option<u32> {
            let span = memory.iter().find(|read| {
                address >= read.begin && address + 4 <= read.begin + read.data.len() as u64
            })?;
            let at = (address - span.begin) as usize;
            let bytes = span.data.get(at..at + 4)?;
            Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        };
        let rows: Vec<RegRow> = peripheral
            .registers
            .iter()
            .map(|register| {
                let value = register
                    .readable
                    .then(|| read(peripheral.base + u64::from(register.offset)))
                    .flatten();
                let open = registers.open.contains(&register.name);
                let fields: Vec<RegField> = if open {
                    register
                        .fields
                        .iter()
                        .map(|field| {
                            let (bit, width) = (field.offset, field.width);
                            let mask = if width >= 32 {
                                u32::MAX
                            } else {
                                (1u32 << width) - 1
                            };
                            RegField {
                                bits: if width == 1 {
                                    format!("[{bit}]")
                                } else {
                                    format!("[{}:{bit}]", bit + width - 1)
                                }
                                .into(),
                                name: field.name.as_str().into(),
                                value: value
                                    .map(|v| ((v >> bit) & mask).to_string())
                                    .unwrap_or_default()
                                    .into(),
                                description: field.description.as_str().into(),
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                RegRow {
                    offset: format!("+0x{:03X}", register.offset).into(),
                    name: register.name.as_str().into(),
                    value: match value {
                        Some(value) => format!("0x{value:08X}"),
                        None if register.readable => "—".to_string(),
                        None => t!("dock.registers.write-only"),
                    }
                    .into(),
                    description: register.description.as_str().into(),
                    open,
                    fields: ModelRc::new(VecModel::from(fields)),
                }
            })
            .collect();
        shell.set_registers_rows(ModelRc::new(VecModel::from(rows)));
    }
}
