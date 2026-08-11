//! Embedded tools.
//!
//! These are the reason an assistant inside rusty beats a general one on this
//! domain. Embedded Rust fails in ways whose error messages point away from the
//! cause: an unsupported-target error that never mentions espup, a linker
//! message that names a region and a byte count and nothing about what filled
//! it. A model reading those strings will produce a fluent, plausible, wrong
//! answer. A model that can call `project_status` and `memory_report` gets the
//! actual cause with the actual numbers.

use serde_json::{Value, json};

use rusty_embed::{memory, project, toolchain};

use super::{Tool, ToolContext, no_arguments, read_only};
use crate::{error::Result, model::ToolDef};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ProjectStatus),
        Box::new(ToolchainStatus),
        Box::new(MemoryReport),
        Box::new(ChipCatalogue),
    ]
}

// ─────────────────────────────────────────────────────────────────────────────

struct ProjectStatus;

impl Tool for ProjectStatus {
    fn def(&self) -> ToolDef {
        read_only(
            "project_status",
            "What the open project targets, read from its Cargo.toml, \
             .cargo/config.toml, and rust-toolchain.toml — the chip, whether it \
             is no_std or ESP-IDF std, the configured target triple and \
             toolchain, which HAL crates are in use — plus a list of problems \
             found by cross-checking those files against each other. \
             \
             Call this before answering any question about why a build fails. \
             These files routinely disagree, and when they do the compiler's \
             error points at none of them: a chip feature saying esp32c3 next \
             to a target triple saying xtensa-esp32-none-elf produces a message \
             about neither. Each problem comes with the reason and, where one \
             exists, the exact command that fixes it.",
            no_arguments(),
        )
    }

    fn call(&self, _args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        let root = ctx.require_root()?;
        Ok(serde_json::to_value(project::detect(root)?)?)
    }
}

// ─────────────────────────────────────────────────────────────────────────────

struct ToolchainStatus;

impl Tool for ToolchainStatus {
    fn def(&self) -> ToolDef {
        read_only(
            "toolchain_status",
            "What Rust and Espressif tooling is installed on this machine — \
             rustup toolchains, installed targets, and whether espup, espflash, \
             probe-rs, esp-generate and ldproxy are present — cross-checked \
             against what the open project needs. \
             \
             Use this for any 'it will not build' or 'it will not flash' \
             question. The classic case is an Xtensa part (ESP32, S2, S3) with \
             a stock toolchain: rustc reports an unknown target and says \
             nothing about espup, so a user can search for a long time without \
             finding the fix. Note that Xtensa targets ship inside the espup \
             toolchain rather than through rustup, so their absence from the \
             installed-target list means nothing.",
            no_arguments(),
        )
    }

    fn call(&self, _args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        // Deliberately works without a project: "is my machine set up?" is a
        // reasonable question before anything is open.
        let detected = ctx.root.and_then(|root| project::detect(root).ok());
        Ok(serde_json::to_value(toolchain::report(detected.as_ref()))?)
    }
}

// ─────────────────────────────────────────────────────────────────────────────

struct MemoryReport;

impl Tool for MemoryReport {
    fn def(&self) -> ToolDef {
        read_only(
            "memory_report",
            "Where the built firmware's bytes went: per-section sizes, total \
             flash and RAM use against the chip's capacity, and — the useful \
             part — how many bytes each crate contributed. \
             \
             Call this for anything about size: a linker error saying a region \
             overflowed, 'why is my binary so large', or which feature to turn \
             off. The linker names a region and a byte count and nothing about \
             the cause; `cargo size` gives section totals, which still does not \
             say which dependency is responsible. This does. \
             \
             Two things worth passing on to the user when they come up: \
             initialised data costs flash *and* RAM, because the initialiser is \
             stored in the image and copied to RAM at startup; and the reported \
             RAM figure is static only — stack and heap grow on top of it, so a \
             comfortable-looking number can still overflow at runtime.",
            no_arguments(),
        )
    }

    fn call(&self, _args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        let firmware = ctx.require_firmware()?;
        let chip_id = ctx
            .root
            .and_then(|root| project::detect(root).ok())
            .and_then(|p| p.chip);
        Ok(serde_json::to_value(memory::analyze(
            firmware,
            chip_id.as_deref(),
        )?)?)
    }
}

// ─────────────────────────────────────────────────────────────────────────────

struct ChipCatalogue;

impl Tool for ChipCatalogue {
    fn def(&self) -> ToolDef {
        read_only(
            "chip_catalogue",
            "The parts and development boards rusty knows about, with \
             architecture, core count, SRAM, radios, the Rust target triple for \
             bare-metal and for std, which toolchain is required, and how each \
             can be flashed. Boards additionally carry flash size, USB identity, \
             and named pins. \
             \
             Use this to answer 'which chip should I pick' and to check a claim \
             about a part before making it. Do not answer these from memory: \
             which Espressif parts are Xtensa versus RISC-V decides whether the \
             user has to install a forked toolchain, and getting it wrong sends \
             them down a long wrong path. \
             \
             The catalogue includes any chips or boards the user or their \
             project added under `.rusty/`, so it is authoritative for them even \
             where it disagrees with what you remember.",
            json!({
                "type": "object",
                "properties": {
                    "chip": {
                        "type": "string",
                        "description": "A single part id, e.g. `esp32c3`. Omit to list all."
                    },
                    "boards": {
                        "type": "boolean",
                        "description": "Include the board list. Boards are what the user physically has."
                    }
                },
                "required": []
            }),
        )
    }

    fn call(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        let catalog = ctx.catalog();
        let want_boards = args.get("boards").and_then(Value::as_bool).unwrap_or(false);

        let Some(wanted) = args.get("chip").and_then(Value::as_str) else {
            let mut out = json!({ "chips": catalog.chips() });
            if want_boards {
                out["boards"] = serde_json::to_value(catalog.boards())?;
            }
            return Ok(out);
        };

        match catalog.chip(wanted) {
            Some(found) => {
                let mut out = serde_json::to_value(found)?;
                // Which boards carry this part is usually the follow-up
                // question, so answering it here saves a round trip.
                out["boards"] = serde_json::to_value(catalog.boards_for_chip(wanted))?;
                Ok(out)
            }
            None => Ok(json!({
                "chip": wanted,
                "known": false,
                "note": "Not in rusty's catalogue. Say so rather than describing it from \
                         memory. The user can add it by writing a TOML file into \
                         `.rusty/chips/` in their project.",
                "available": catalog.chips().iter().map(|c| &c.id).collect::<Vec<_>>(),
            })),
        }
    }
}
