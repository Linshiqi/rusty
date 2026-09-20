//! The firmware, run.
//!
//! Every other tool here reads: a manifest, a linker map, a catalogue. This
//! one builds the project, boots it in rusty's emulator and reports what it
//! did — what it printed, which pins moved, what crossed the buses. It exists
//! for the question the others can only answer by inference: "does it do
//! what I think it does?" A model asked why an LED never lights reads the
//! code and proposes a cause; with this it can watch GPIO2 stay low, press
//! the button on the sheet, and watch again.
//!
//! It runs commands (cargo, espflash, QEMU), and cargo may fetch crates, so
//! it is declared that way and served only where the client asks the user
//! first — the MCP server's clients do. The built-in assistant has no
//! approval step yet, and does not get it.

use serde_json::{Value, json};

use rusty_embed::project;
use rusty_embed::simulate::headless::{self, Scenario, Verdict};

use super::{Tool, ToolContext};
use crate::error::{Error, Result};
use crate::model::{Capabilities, ToolDef, ToolSource};

/// The longest a run may be asked for. A model that wants a minute of a
/// control loop can have it; one that asks for an hour is holding a client
/// open for no one.
const LONGEST: f64 = 60.0;

/// How much of what the firmware printed goes back: the end, which is where
/// a run that went wrong went wrong.
const SERIAL_LINES: usize = 200;
const BUS_LINES: usize = 40;

pub(super) struct Simulate;

impl Tool for Simulate {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "simulate".to_string(),
            description: "Build the open project's firmware, boot it in rusty's emulator \
                (Espressif's QEMU with rusty's pin, ADC, I2C, SPI, LEDC and RMT models), \
                and report what it did: every line it printed, the level of every GPIO \
                that moved, and everything that crossed a peripheral — I2C and SPI \
                transactions, a duty with the frequency its timer sets, the bytes a LED \
                strip was sent. The board drawn in the project's .rusty/sim.toml is on \
                the pins and buses — its buttons, knobs, sensors and screen. \
                \
                Call this instead of reasoning about what firmware will print or whether a \
                pin toggles: code that reads correctly and a board that behaves are two \
                different facts, and only the second can be watched. Steps drive the \
                board while it runs, in order, each object carrying exactly one of: \
                wait-serial (text to wait for), write-serial (a line into the console), \
                press / release (a switch's reference such as \"SW1\", or a GPIO number), \
                delay (seconds), expect-pin ({gpio, level}), set ({part, <reading>: value} \
                for a sensor with a model, e.g. {\"part\": \"U2\", \"ax\": 0.5}). \
                `expect` lists text the run must see to pass; `fail` text ends it as a \
                failure. The first build can take minutes; the timeout counts from boot. \
                Supported on ESP32-C3; on an ESP32 or S3 the answer says what the emulator \
                cannot do there."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "timeout": {
                        "type": "number",
                        "description": "Seconds the firmware may run once booted: default 10, at most 60."
                    },
                    "expect": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Text the firmware must print; the run passes once all of it has appeared and every step is done."
                    },
                    "fail": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Text that fails the run the moment it appears, such as \"panicked\"."
                    },
                    "steps": {
                        "type": "array",
                        "items": { "type": "object" },
                        "description": "Steps taken in order while the firmware runs, each with exactly one action key."
                    }
                },
                "required": []
            }),
            capabilities: Capabilities {
                reads_workspace: true,
                // What it writes is build output under target/, as any build
                // does; the project's own files are untouched.
                writes_workspace: false,
                network: true,
                runs_commands: true,
            },
            source: ToolSource::Builtin,
        }
    }

    fn call(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        let root = ctx.require_root()?;
        let mut scenario: Scenario =
            serde_json::from_value(args.clone()).map_err(|error| Error::BadToolArguments {
                name: "simulate".to_string(),
                detail: error.to_string(),
            })?;
        scenario.check().map_err(|detail| Error::BadToolArguments {
            name: "simulate".to_string(),
            detail,
        })?;
        scenario.timeout = Some(
            scenario
                .timeout
                .unwrap_or(headless::DEFAULT_TIMEOUT)
                .min(LONGEST),
        );

        let firmware = project::firmware_root(root);
        let outcome = headless::run(&firmware, &scenario, &mut |_| {});
        let (verdict, reason) = match &outcome.verdict {
            Verdict::Passed => ("passed", None),
            Verdict::Failed(why) => ("failed", Some(why.clone())),
            Verdict::TimedOut(why) => ("timed out", Some(why.clone())),
            Verdict::Unrunnable(why) => return Err(Error::Refused(why.clone())),
        };

        let total = outcome.serial.len();
        let serial: Vec<&String> = outcome
            .serial
            .iter()
            .skip(total.saturating_sub(SERIAL_LINES))
            .collect();
        let mut pins = serde_json::Map::new();
        for (pin, level) in outcome.levels() {
            let reports = outcome.events.iter().filter(|(_, p, _)| *p == pin).count();
            pins.insert(
                format!("GPIO{pin}"),
                json!({ "level": u8::from(level), "reports": reports }),
            );
        }
        let bus_total = outcome.bus.len();
        let bus: Vec<&String> = outcome
            .bus
            .iter()
            .skip(bus_total.saturating_sub(BUS_LINES))
            .collect();
        Ok(json!({
            "verdict": verdict,
            "reason": reason,
            "serial": serial,
            "serialTotal": total,
            "serialTruncated": total > SERIAL_LINES,
            "pins": pins,
            "pinsFromEmulator": outcome.pins_from_emulator,
            "bus": bus,
            "busTotal": bus_total,
            "limits": outcome.limits.iter().map(|limit| &limit.text).collect::<Vec<_>>(),
            "notes": outcome.notes,
        }))
    }
}
