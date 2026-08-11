//! The workbench's analyses, exposed as tools a model can call.
//!
//! This is what "AI native" means here in practice. Other assistants faced with
//! an embedded Rust project have to guess: grep the manifest, read a linker
//! error as prose, infer which chip is in play. rusty *computes* those answers.
//! So the assistant does not read `.cargo/config.toml` and speculate about why
//! a build fails — it calls `project_status` and gets the actual mismatch, with
//! the fix.
//!
//! The same definitions are intended to back three consumers: the built-in
//! assistant, an MCP server (so Claude Code and Cursor users get these analyses
//! too), and the CLI. Adding an analysis should mean adding a tool here, not
//! wiring three integrations.

mod cargo;
mod context;
mod embedded;

use serde_json::Value;

use crate::{
    error::{Error, Result},
    // The definitions live in `model` so the UI can show what the assistant is
    // allowed to do without linking any of the machinery that does it.
    model::{ToolDef, ToolSource},
};

pub use context::ToolContext;

pub trait Tool: Send + Sync {
    fn def(&self) -> ToolDef;
    fn call(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<Value>;
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    /// The built-in set: everything the workbench can currently answer.
    pub fn workbench() -> Self {
        let mut tools = cargo::tools();
        tools.extend(embedded::tools());
        Self { tools }
    }

    /// Add a tool from outside the built-in set.
    ///
    /// The extension seam. A tool arriving here is namespaced by its source, so
    /// a connected MCP server can never shadow `project_status` — name
    /// collisions between a host tool and a third-party one are otherwise a
    /// silent, and quite exploitable, failure.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn defs(&self) -> Vec<ToolDef> {
        self.tools
            .iter()
            .map(|t| {
                let mut def = t.def();
                def.name = qualified_name(&def);
                def
            })
            .collect()
    }

    pub fn call(&self, name: &str, args: &Value, ctx: &ToolContext<'_>) -> Result<Value> {
        self.tools
            .iter()
            .find(|t| qualified_name(&t.def()) == name)
            .ok_or_else(|| Error::UnknownTool(name.to_string()))?
            .call(args, ctx)
    }

    /// True when nothing in the registry needs user approval, which is what
    /// lets the assistant run its whole loop without interrupting.
    pub fn is_read_only(&self) -> bool {
        self.tools
            .iter()
            .all(|t| !t.def().capabilities.needs_approval())
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::workbench()
    }
}

/// Built-ins keep their bare names; everything else is prefixed by its source.
fn qualified_name(def: &ToolDef) -> String {
    match &def.source {
        ToolSource::Builtin => def.name.clone(),
        ToolSource::Mcp { server } => format!("mcp__{server}__{}", def.name),
    }
}

// ─── shared argument helpers ─────────────────────────────────────────────────

pub(crate) fn required_str(args: &Value, key: &str, tool: &str) -> Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::BadToolArguments {
            name: tool.to_string(),
            detail: format!("`{key}` is required and must be a string"),
        })
}

pub(crate) fn string_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// A tool taking no arguments, spelled the way both wire formats expect.
pub(crate) fn no_arguments() -> Value {
    serde_json::json!({ "type": "object", "properties": {}, "required": [] })
}

/// Declare a read-only tool.
///
/// Every built-in is read-only, so this is the only constructor either tool
/// module needs — and having one means a tool cannot acquire write capability
/// by being declared in the file that happens to allow it.
pub(crate) fn read_only(name: &str, description: &str, input_schema: Value) -> ToolDef {
    ToolDef {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
        capabilities: crate::model::Capabilities::READ_ONLY,
        source: ToolSource::Builtin,
    }
}
