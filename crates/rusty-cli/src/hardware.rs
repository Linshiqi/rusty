//! `catalog`, `devices` and `symbol`: the parts rusty knows, the ones
//! plugged in, and one fetched from LCSC.

use std::path::Path;

use anyhow::Result;
use rusty_embed::{catalog::Catalog, device};

use super::{emit, human};

/// `rusty catalog`: the parts and boards rusty knows, the project's own
/// included.
pub(crate) fn catalog(path: &Path, boards: bool, json: bool) -> Result<()> {
    let catalog = Catalog::load(Some(path));
    report_catalog_problems(&catalog);

    if json && boards {
        emit(&catalog.boards())?;
    } else if json {
        emit(&catalog.chips())?;
    } else if boards {
        for board in catalog.boards() {
            let flash = board
                .flash_bytes
                .map(|b| format!("{} flash", human(b as u64)))
                .unwrap_or_else(|| "flash unknown".into());
            println!(
                "  {:<28} {:<10} {:<14} [{}]",
                board.name,
                board.chip,
                flash,
                board.source.label()
            );
        }
    } else {
        for chip in catalog.chips() {
            println!(
                "  {:<10} {:<24} {:<14} {}",
                chip.id,
                chip.name,
                chip.arch.label(),
                chip.bare_metal_target
            );
        }
    }
    Ok(())
}

/// `rusty devices`: the serial ports and debug probes attached now.
pub(crate) fn devices(path: &Path, json: bool) -> Result<()> {
    let catalog = Catalog::load(Some(path));
    let ports = device::list_serial_ports(&catalog);
    let probes = device::list_probes();

    if json {
        emit(&serde_json::json!({ "ports": ports, "probes": probes }))?;
    } else {
        if ports.is_empty() {
            println!("no serial ports");
        }
        for port in &ports {
            // The board name is what the user recognises; the bridge
            // chip is the fallback when nothing in the catalogue matches.
            let what = if !port.boards.is_empty() {
                port.boards.join(" / ")
            } else {
                port.bridge.clone().unwrap_or_else(|| "unknown".into())
            };
            println!("  {:<12} {}", port.name, what);
        }
        for probe in &probes {
            println!("  probe        {}", probe.description);
        }
    }
    Ok(())
}

/// `rusty symbol`: an LCSC part as a schematic symbol, imported from
/// EasyEDA.
pub(crate) fn symbol(number: &str, json: bool) -> Result<()> {
    let imported = rusty_embed::schematic::easyeda::import(number)?;
    if json {
        emit(&imported.symbol)?;
    } else {
        print_symbol(&imported.symbol);
    }
    for warning in &imported.warnings {
        eprintln!("{warning}");
    }
    Ok(())
}

fn print_symbol(symbol: &rusty_embed::Symbol) {
    println!(
        "{}  reference {}  value {}",
        symbol.id(),
        symbol.reference,
        symbol.value
    );
    if let Some(description) = &symbol.description {
        println!("  {description}");
    }
    for pin in &symbol.pins {
        println!(
            "  pin {:<4} {:<12} {:<14} at ({}, {}) mm, {} mm toward {}°{}",
            pin.number,
            pin.name,
            format!("{:?}", pin.kind).to_lowercase(),
            pin.at.0,
            pin.at.1,
            pin.length,
            pin.angle,
            if pin.hidden { "  (hidden)" } else { "" }
        );
    }
    println!("  {} graphics", symbol.graphics.len());
}

fn report_catalog_problems(catalog: &Catalog) {
    // To stderr, so `--json` output stays machine-readable while a broken board
    // file still gets noticed.
    for problem in catalog.problems() {
        eprintln!("warning: {} — {}", problem.path, problem.detail);
    }
}
