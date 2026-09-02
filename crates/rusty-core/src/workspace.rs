use std::{path::Path, process::Command};

use camino::Utf8Path;
use guppy::{MetadataCommand, graph::PackageGraph};

use crate::{
    duplicates,
    error::{Error, Result},
    features,
    model::*,
    overview,
};

/// A loaded Cargo workspace.
///
/// Loading runs `cargo metadata` once and builds the package + feature graphs;
/// every analysis after that is in-memory, which is what makes the live feature
/// matrix possible.
pub struct Workspace {
    graph: PackageGraph,
    host_triple: String,
}

impl Workspace {
    /// Load the workspace containing `path`.
    ///
    /// `path` may be the workspace root, any member directory, or a
    /// `Cargo.toml` directly — cargo walks up to find the real root either way.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let manifest = if path.is_dir() {
            path.join("Cargo.toml")
        } else {
            path.to_path_buf()
        };

        let mut cmd = MetadataCommand::new();
        cmd.manifest_path(&manifest);
        // Run cargo ourselves rather than through guppy's builder: the flag
        // that keeps a Windows child from opening a console window lives on
        // `std::process::Command`, and a GUI parent that let guppy spawn
        // `cargo metadata` flashed one up on every project open.
        let mut command = cmd.cargo_command();
        quiet(&mut command);
        let output = command.output().map_err(|e| Error::CargoRun {
            path: manifest.display().to_string(),
            detail: e.to_string(),
        })?;
        if !output.status.success() {
            return Err(Error::CargoRun {
                path: manifest.display().to_string(),
                detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        let json = String::from_utf8_lossy(&output.stdout);
        let graph = guppy::CargoMetadata::parse_json(&json)
            .and_then(|metadata| metadata.build_graph())
            .map_err(|e| Error::metadata(manifest.display(), e))?;

        Ok(Self {
            graph,
            host_triple: host_triple(),
        })
    }

    pub fn graph(&self) -> &PackageGraph {
        &self.graph
    }

    pub fn root(&self) -> &Utf8Path {
        self.graph.workspace().root()
    }

    /// Everything the Overview page renders, in one pass.
    pub fn report(&self) -> Result<WorkspaceReport> {
        let duplicates = duplicates::analyze(&self.graph);
        let members = overview::members(&self.graph)?;
        let vitals = overview::vitals(&self.graph, &members, &duplicates);
        let workspace = self.info();

        Ok(WorkspaceReport {
            workspace,
            vitals,
            members,
            duplicates,
        })
    }

    fn info(&self) -> WorkspaceInfo {
        let ws = self.graph.workspace();
        let root = ws.root();

        // Highest edition and MSRV declared by any member — that is what the
        // workspace effectively requires, not what any single crate says.
        let edition = ws.iter().map(|p| p.edition().to_string()).max();
        let rust_version = ws
            .iter()
            .filter_map(|p| p.minimum_rust_version())
            .max()
            .map(|v| v.to_string());

        let name = ws
            .iter()
            .find(|p| p.manifest_path().parent() == Some(root))
            .map(|p| p.name().to_string())
            .or_else(|| root.file_name().map(str::to_string))
            .unwrap_or_else(|| root.to_string());

        WorkspaceInfo {
            root: root.to_string(),
            name,
            edition,
            rust_version,
            target_platform: self.host_triple.clone(),
        }
    }

    /// Simulate one feature selection and report what it costs relative to the
    /// package's defaults. See [`features::impact`].
    pub fn feature_impact(&self, selection: &FeatureSelection) -> Result<FeatureImpact> {
        features::impact(&self.graph, selection)
    }

    /// Every feature a workspace member declares, with the marginal cost of
    /// each one given the rest of `selection`. Backs the feature matrix rows.
    pub fn feature_rows(&self, selection: &FeatureSelection) -> Result<Vec<FeatureRow>> {
        features::rows(&self.graph, selection)
    }
}

/// No console window for a child on Windows. A GUI process has no console,
/// so a child that inherits nothing is given one — a black window flashed up
/// for the duration of every `cargo metadata` and `rustc -vV`. rusty-embed
/// carries the same three lines; this crate cannot depend on it.
fn quiet(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// Ask rustc for the host triple.
///
/// `std::env::consts` cannot produce a real triple (it has no vendor or ABI),
/// and getting this wrong would mislabel every analysis, so shell out to the
/// authority. Falls back to a clearly-marked placeholder rather than a wrong
/// triple if rustc is not on PATH.
fn host_triple() -> String {
    let mut command = Command::new("rustc");
    command.arg("-vV");
    quiet(&mut command);
    command
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| {
            String::from_utf8(out.stdout).ok().and_then(|text| {
                text.lines()
                    .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
            })
        })
        .unwrap_or_else(|| "unknown".to_string())
}
