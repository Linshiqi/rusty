//! Cargo dependency and feature tools.
//!
//! Useful on their own, and more so on an embedded project than on a server
//! one: feature selection there is not a tidiness question, it decides whether
//! the binary fits in flash.

use serde_json::{Value, json};

use rusty_core::FeatureSelection;

use super::{Tool, ToolContext, no_arguments, read_only, required_str, string_list};
use crate::{error::Result, model::ToolDef};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(WorkspaceReport),
        Box::new(ExplainDuplicate),
        Box::new(SimulateFeatures),
        Box::new(ListFeatures),
    ]
}

/// A feature selection built from loosely-typed tool arguments.
fn selection(args: &Value, tool: &str) -> Result<FeatureSelection> {
    Ok(FeatureSelection {
        package: required_str(args, "package", tool)?,
        features: string_list(args, "features"),
        default_features: args
            .get("defaultFeatures")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

fn selection_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "package": { "type": "string", "description": "Workspace member to resolve for." },
            "features": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Features to enable, as in `cargo --features`."
            },
            "defaultFeatures": {
                "type": "boolean",
                "description": "False is equivalent to `--no-default-features`. Defaults to true."
            }
        },
        "required": ["package"]
    })
}

// ─────────────────────────────────────────────────────────────────────────────

struct WorkspaceReport;

impl Tool for WorkspaceReport {
    fn def(&self) -> ToolDef {
        read_only(
            "workspace_report",
            "Health report for the open Cargo workspace: crate counts, how many \
             dependencies resolved (direct vs transitive), which crates resolved \
             to multiple versions, build-script and proc-macro counts, the \
             effective MSRV, and a row per workspace member. Computed from the \
             real resolved dependency graph, not inferred from manifests.",
            no_arguments(),
        )
    }

    fn call(&self, _args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        Ok(serde_json::to_value(ctx.require_workspace()?.report()?)?)
    }
}

// ─────────────────────────────────────────────────────────────────────────────

struct ExplainDuplicate;

impl Tool for ExplainDuplicate {
    fn def(&self) -> ToolDef {
        read_only(
            "explain_duplicate",
            "Explain why a crate appears at more than one version. Returns each \
             resolved version, exactly which packages requested it and with what \
             version requirement, whether the requester is one of the user's own \
             crates, and whether the versions are semver-compatible (so a \
             `cargo update` could unify them) or genuinely incompatible (so a \
             dependency has to move first). Use this instead of reasoning about \
             version numbers yourself.",
            json!({
                "type": "object",
                "properties": {
                    "crate": {
                        "type": "string",
                        "description": "Crate name, e.g. `base64`. Omit to list every duplicated crate."
                    }
                },
                "required": []
            }),
        )
    }

    fn call(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        let all = ctx.require_workspace()?.report()?.duplicates;

        let Some(wanted) = args.get("crate").and_then(Value::as_str) else {
            return Ok(json!({
                "duplicatedCrates": all.iter().map(|g| &g.name).collect::<Vec<_>>(),
                "count": all.len(),
            }));
        };

        match all.into_iter().find(|g| g.name == wanted) {
            Some(group) => Ok(serde_json::to_value(group)?),
            None => Ok(json!({
                "crate": wanted,
                "duplicated": false,
                "note": "This crate resolved to a single version, or is not in the dependency graph."
            })),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────

struct SimulateFeatures;

impl Tool for SimulateFeatures {
    fn def(&self) -> ToolDef {
        read_only(
            "simulate_features",
            "Simulate a Cargo feature selection and report what it actually \
             costs: how many crates resolve, the delta against that package's \
             defaults, and the exact crates added and removed. The simulation \
             replays Cargo's resolver over the whole workspace, so feature \
             unification is already applied — turning a feature off only drops \
             a crate if nothing else still needs it. \
             \
             Never estimate this from manifest text; the manifest cannot tell \
             you the answer. On an embedded project this is also the fastest \
             route to shrinking a binary that will not fit.",
            selection_schema(),
        )
    }

    fn call(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        let workspace = ctx.require_workspace()?;
        let impact = workspace.feature_impact(&selection(args, "simulate_features")?)?;
        Ok(serde_json::to_value(impact)?)
    }
}

// ─────────────────────────────────────────────────────────────────────────────

struct ListFeatures;

impl Tool for ListFeatures {
    fn def(&self) -> ToolDef {
        read_only(
            "list_features",
            "Every feature a package declares, with whether it is currently on, \
             whether `default` enables it, which sibling features it turns on, \
             and what flipping that one switch would cost in crates. \
             \
             Two coupled features can each show zero because either one alone \
             keeps the shared dependency alive — that is real, not a bug, and \
             worth explaining to the user when it happens.",
            selection_schema(),
        )
    }

    fn call(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        let workspace = ctx.require_workspace()?;
        let rows = workspace.feature_rows(&selection(args, "list_features")?)?;
        Ok(json!({ "features": rows }))
    }
}
