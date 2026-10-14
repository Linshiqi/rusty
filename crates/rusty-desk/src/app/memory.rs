//! The Memory page: where the image's bytes went.
//!
//! Exists for the failure that ends an afternoon: the linker says a region
//! overflowed by three kilobytes and names no cause. Per-section totals
//! restate the situation; the question is always *what is costing me 40 KB*,
//! and only per-crate attribution answers it — so the crate table gets the
//! room, and the sections are below it.
//!
//! Every build the project has is listed, always — `debug` and `release`
//! differ by enough that numbers with nothing saying which binary made them
//! are dangerous — and one built for another triple than the project's is
//! marked, because it flashes cleanly and then behaves like a hardware fault.

use std::{path::Path, rc::Rc};

use rusty_embed::{Firmware, MemoryReport, SectionKind};
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::{App, Tab};
use crate::{MemBuild, MemCrate, MemSection, Shell, format, work};

/// Crates listed before the rest are counted: a real firmware's interesting
/// crates fit, and the rest are rounding error whose total is stated.
const SHOWN: usize = 15;

#[derive(Default)]
pub(super) struct Memory {
    builds: Vec<Firmware>,
    chosen: Option<String>,
    report: Option<MemoryReport>,
    problem: Option<String>,
    expanded: bool,
    /// The builds have been listed at least once.
    listed: bool,
    asked: u64,
}

impl App {
    pub(super) fn memory_open(self: &Rc<Self>) {
        self.open_tab(Tab::Page("memory"));
        self.memory_list();
    }

    /// The builds listed again, and the one chosen measured.
    pub(super) fn memory_list(self: &Rc<Self>) {
        let Some(root) = self.root() else {
            self.sync_memory();
            return;
        };
        let asked = {
            let mut state = self.state.borrow_mut();
            state.memory.asked += 1;
            state.memory.asked
        };
        work::spawn(
            move || {
                let firmware = rusty_embed::project::firmware_root(&root);
                let detected = rusty_embed::project::detect(&firmware).ok();
                let builds = match &detected {
                    Some(project) => rusty_embed::firmware::list_project(&firmware, project),
                    None => rusty_embed::firmware::list(&firmware, None),
                };
                (builds, detected.and_then(|p| p.chip), firmware)
            },
            move |app, (builds, chip, firmware)| {
                let path = {
                    let mut state = app.state.borrow_mut();
                    let memory = &mut state.memory;
                    if memory.asked != asked {
                        return;
                    }
                    memory.listed = true;
                    // The build chosen stays chosen while it exists; the
                    // newest built for the project's own target otherwise.
                    let keep = memory
                        .chosen
                        .as_ref()
                        .filter(|chosen| builds.iter().any(|b| &&b.path == chosen))
                        .cloned();
                    memory.chosen = keep.or_else(|| {
                        builds
                            .iter()
                            .find(|b| b.matches_configured_target)
                            .or(builds.first())
                            .map(|b| b.path.clone())
                    });
                    memory.builds = builds;
                    memory.chosen.clone()
                };
                app.sync_memory();
                if let Some(path) = path {
                    app.memory_measure(path, chip, firmware);
                } else {
                    app.state.borrow_mut().memory.report = None;
                    app.sync_memory();
                }
            },
        );
    }

    fn memory_measure(
        self: &Rc<Self>,
        path: String,
        chip: Option<String>,
        firmware: std::path::PathBuf,
    ) {
        let asked = self.state.borrow().memory.asked;
        work::spawn(
            move || {
                rusty_embed::memory::analyze_project(
                    Path::new(&path),
                    chip.as_deref(),
                    Some(&firmware),
                )
                .map_err(|e| e.to_string())
            },
            move |app, report| {
                {
                    let mut state = app.state.borrow_mut();
                    let memory = &mut state.memory;
                    if memory.asked != asked {
                        return;
                    }
                    match report {
                        Ok(report) => {
                            memory.report = Some(report);
                            memory.problem = None;
                        }
                        Err(why) => {
                            memory.report = None;
                            memory.problem = Some(why);
                        }
                    }
                }
                app.sync_memory();
            },
        );
    }

    /// A build picked from the row of them.
    pub(super) fn memory_pick(self: &Rc<Self>, path: &str) {
        self.state.borrow_mut().memory.chosen = Some(path.to_string());
        self.memory_list();
    }

    pub(super) fn memory_act(self: &Rc<Self>, verb: &str) {
        match verb {
            "open" => self.memory_open(),
            "reread" => self.memory_list(),
            "more" => {
                {
                    let mut state = self.state.borrow_mut();
                    state.memory.expanded = !state.memory.expanded;
                }
                self.sync_memory();
            }
            _ => {}
        }
    }

    /// A build finished: the page, if it has been looked at, measures the
    /// new image.
    pub(super) fn memory_after_build(self: &Rc<Self>) {
        if self.state.borrow().memory.listed {
            self.memory_list();
        }
    }

    pub(super) fn sync_memory(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let memory = &state.memory;
        let has_project = state.project.is_some();
        shell.set_mem_state(if !has_project {
            0
        } else if !memory.listed {
            3
        } else if memory.builds.is_empty() {
            1
        } else {
            2
        });
        let now = jiff::Timestamp::now().as_second().max(0) as u64;
        let builds: Vec<MemBuild> = memory
            .builds
            .iter()
            .map(|b| MemBuild {
                path: b.path.as_str().into(),
                name: b.name.as_str().into(),
                profile: b.profile.as_str().into(),
                size: format::bytes(b.bytes).into(),
                age: b
                    .modified
                    .map(|m| format::since(m, now))
                    .unwrap_or_default()
                    .into(),
                stale: !b.matches_configured_target,
                target: b.target.as_str().into(),
                chosen: memory.chosen.as_deref() == Some(b.path.as_str()),
            })
            .collect();
        shell.set_mem_builds(ModelRc::new(VecModel::from(builds)));
        shell.set_mem_problem(memory.problem.clone().unwrap_or_default().into());
        let Some(report) = &memory.report else {
            shell.set_mem_ready(false);
            return;
        };
        shell.set_mem_ready(true);
        let totals = &report.totals;
        let (value, unit) = format::bytes_parts(totals.flash_bytes);
        shell.set_mem_flash_value(value.into());
        shell.set_mem_flash_unit(unit.into());
        let flash = totals.flash_fraction();
        shell.set_mem_flash_tone(tone(flash, 0.75));
        shell.set_mem_flash_hint(
            match (flash, totals.flash_capacity) {
                (Some(f), Some(capacity)) => t!(
                    "memory.flash-of",
                    percent = format::percent(f),
                    capacity = format::bytes(u64::from(capacity))
                ),
                _ => t!("memory.flash-hint"),
            }
            .into(),
        );
        let (value, unit) = format::bytes_parts(totals.ram_bytes);
        shell.set_mem_ram_value(value.into());
        shell.set_mem_ram_unit(unit.into());
        let ram = totals.ram_fraction();
        // Nominal SRAM, not what the linker will grant: anything nearing it
        // is already trouble, so the amber starts well below full.
        shell.set_mem_ram_tone(tone(ram, 0.7));
        shell.set_mem_ram_hint(
            match (ram, totals.ram_capacity) {
                (Some(f), Some(capacity)) => t!(
                    "desk.memory-ram-of",
                    percent = format::percent(f),
                    capacity = format::bytes(u64::from(capacity))
                ),
                _ => t!("memory.capacity-unknown"),
            }
            .into(),
        );
        let (value, unit) = format::bytes_parts(report.unattributed_bytes);
        shell.set_mem_unattributed(value.into());
        shell.set_mem_unattributed_unit(unit.into());
        // Bars against the largest crate rather than the total: against the
        // total every row of a healthy binary is a sliver.
        let largest = report.crates.first().map(|c| c.total).unwrap_or(0).max(1);
        let limit = if memory.expanded {
            report.crates.len()
        } else {
            SHOWN
        };
        let crates: Vec<MemCrate> = report
            .crates
            .iter()
            .take(limit)
            .map(|c| MemCrate {
                name: c.name.as_str().into(),
                code: format::bytes(c.code).into(),
                rodata: format::bytes(c.read_only_data).into(),
                ram: format::bytes(c.data + c.bss).into(),
                total: format::bytes(c.total).into(),
                bar: c.total as f32 / largest as f32,
            })
            .collect();
        shell.set_mem_crates(ModelRc::new(VecModel::from(crates)));
        let hidden = report.crates.len().saturating_sub(SHOWN);
        let hidden_bytes: u64 = report.crates.iter().skip(SHOWN).map(|c| c.total).sum();
        shell.set_mem_more(
            if hidden == 0 {
                String::new()
            } else if memory.expanded {
                t!("memory.show-fewer")
            } else {
                t!(
                    "memory.show-more",
                    count = hidden,
                    size = format::bytes(hidden_bytes)
                )
            }
            .into(),
        );
        let attributed: u64 = report.crates.iter().map(|c| c.total).sum();
        shell.set_mem_attributed(
            format!("{}{}", format::bytes(attributed), t!("memory.attributed")).into(),
        );
        let sections: Vec<MemSection> = report
            .sections
            .iter()
            .map(|s| {
                let costs = match s.kind.budget() {
                    (true, true) => t!("memory.cost-both"),
                    (true, false) => t!("memory.cost-flash"),
                    (false, true) => t!("memory.cost-ram"),
                    (false, false) => "—".to_string(),
                };
                MemSection {
                    name: s.name.as_str().into(),
                    kind: s.kind.label().into(),
                    tone: match s.kind {
                        SectionKind::Code => 0,
                        SectionKind::ReadOnlyData => 1,
                        SectionKind::InitialisedData => 2,
                        SectionKind::ZeroedData => 3,
                    },
                    costs: costs.into(),
                    address: format!("{:#010x}", s.address).into(),
                    size: format::bytes(s.size).into(),
                }
            })
            .collect();
        shell.set_mem_sections(ModelRc::new(VecModel::from(sections)));
        shell.set_mem_elf(report.elf_path.as_str().into());
    }
}

/// A fraction's colour: 0 neutral, 1 fine, 2 near, 3 over the red line.
fn tone(fraction: Option<f32>, amber: f32) -> i32 {
    match fraction {
        Some(f) if f >= 0.9 => 3,
        Some(f) if f >= amber => 2,
        Some(_) => 1,
        None => 0,
    }
}
