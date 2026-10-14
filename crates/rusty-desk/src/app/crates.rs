//! The Crates page: what the workspace pulls in and what it costs.
//!
//! One page, four questions asked in the order people ask them — what do I
//! depend on, and is it current (against crates.io, asked only when the page
//! asks, since it is one index request per crate); why are there two of
//! something (the duplicate versions, with who pulled each in); what does a
//! feature cost (a switch per feature, flipped against the whole graph); and
//! what all of it costs on disk (`rusty_core::disk`: the build trees, what in
//! them is stale and why, and the removals that are safe).
//!
//! The Cargo analysis is `cargo metadata` resolved into a graph — unbounded
//! on a project whose lockfile does not exist yet — so it is loaded the first
//! time the page is shown and kept until the manifest changes under it.

use std::{path::PathBuf, rc::Rc, sync::Arc};

use rusty_core::{
    CrateRow, DiskKind, DiskReport, FeatureImpact, FeatureRow, FeatureSelection, SweepPolicy,
    SweepReport, Workspace, WorkspaceReport,
};
use rusty_embed::CommandPlan;
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{App, Tab, git::confirm};
use crate::{
    CrateDep, CrateDup, CrateVital, DiskIdle, DiskItemRow, DiskTree, FeatPackage, FeatRow, Shell,
    format, run, work,
};

/// The idle thresholds offered, in days.
const IDLE: [u32; 4] = [1, 7, 30, 90];

pub(super) struct Crates {
    root: Option<PathBuf>,
    workspace: Option<Arc<Workspace>>,
    loading: bool,
    problem: Option<String>,
    report: Option<WorkspaceReport>,
    registry: Option<Vec<CrateRow>>,
    asking: bool,
    selection: Option<FeatureSelection>,
    features: Vec<FeatureRow>,
    impact: Option<FeatureImpact>,
    disk: Option<DiskReport>,
    disk_busy: bool,
    idle_days: u32,
}

impl Default for Crates {
    fn default() -> Self {
        Crates {
            root: None,
            workspace: None,
            loading: false,
            problem: None,
            report: None,
            registry: None,
            asking: false,
            selection: None,
            features: Vec::new(),
            impact: None,
            disk: None,
            disk_busy: false,
            idle_days: 7,
        }
    }
}

impl App {
    pub(super) fn crates_act(self: &Rc<Self>, verb: &str) {
        match verb {
            "open" => {
                self.open_tab(Tab::Page("crates"));
                self.crates_shown();
            }
            "shown" => self.crates_shown(),
            "reload" => self.crates_reload(),
            "ask" => self.crates_ask(),
            "defaults" => self.features_defaults(),
            "disk-scan" => self.disk_scan(),
            "disk-sweep" => self.disk_sweep(None),
            _ => {}
        }
    }

    /// The page on screen: the analysis, loaded once per project.
    fn crates_shown(self: &Rc<Self>) {
        let root = self.root();
        let fresh = {
            let state = self.state.borrow();
            state.crates.root != root
        };
        if fresh {
            self.state.borrow_mut().crates = Crates {
                root: root.clone(),
                ..Crates::default()
            };
        }
        let needed = {
            let state = self.state.borrow();
            state.crates.workspace.is_none() && !state.crates.loading && root.is_some()
        };
        if needed {
            self.crates_load();
        }
        let scan = self.state.borrow().crates.disk.is_none();
        if scan {
            self.disk_scan();
        }
        self.sync_crates();
    }

    /// The manifest changed: the analysis read again.
    fn crates_reload(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            state.crates.workspace = None;
            state.crates.report = None;
            state.crates.registry = None;
        }
        self.crates_load();
    }

    fn crates_load(self: &Rc<Self>) {
        let Some(root) = self.root() else { return };
        self.state.borrow_mut().crates.loading = true;
        self.sync_crates();
        work::spawn(
            move || {
                let workspace = Workspace::load(&root).map_err(|e| e.to_string())?;
                let report = workspace.report().map_err(|e| e.to_string())?;
                Ok::<_, String>((root, Arc::new(workspace), report))
            },
            |app, loaded| {
                let selection = {
                    let mut state = app.state.borrow_mut();
                    let crates = &mut state.crates;
                    crates.loading = false;
                    match loaded {
                        Ok((root, workspace, report)) => {
                            if crates.root.as_ref() != Some(&root) {
                                return;
                            }
                            crates.problem = None;
                            // The member with the most features is the one a
                            // feature question is most likely about.
                            let keep = crates
                                .selection
                                .clone()
                                .filter(|s| report.members.iter().any(|m| m.name == s.package));
                            let selection =
                                keep.or_else(|| {
                                    report.members.iter().max_by_key(|m| m.features.len()).map(
                                        |m| FeatureSelection {
                                            package: m.name.clone(),
                                            features: Vec::new(),
                                            default_features: true,
                                        },
                                    )
                                });
                            crates.workspace = Some(workspace);
                            crates.report = Some(report);
                            selection
                        }
                        Err(why) => {
                            crates.problem = Some(why);
                            None
                        }
                    }
                };
                app.sync_crates();
                if let Some(selection) = selection {
                    app.features_apply(selection);
                }
            },
        );
    }

    /// Every direct dependency against crates.io. Slow by nature, so only
    /// the page's own button asks.
    fn crates_ask(self: &Rc<Self>) {
        let Some(workspace) = self.state.borrow().crates.workspace.clone() else {
            return;
        };
        self.state.borrow_mut().crates.asking = true;
        self.sync_crates();
        work::spawn(
            move || {
                let deps = rusty_core::registry::direct_dependencies(workspace.graph());
                rusty_core::registry::annotate_latest(deps, rusty_embed::net::effective_proxy())
            },
            |app, rows| {
                {
                    let mut state = app.state.borrow_mut();
                    state.crates.asking = false;
                    state.crates.registry = Some(rows);
                }
                app.sync_crates();
            },
        );
    }

    /// `cargo add name@version` in the dock, then the analysis read again:
    /// the manifest changed, so the old graph is stale.
    pub(super) fn crates_upgrade(self: &Rc<Self>, name: &str, version: &str) {
        if version.is_empty() {
            return;
        }
        let plan = CommandPlan::new(
            "cargo",
            vec!["add".into(), format!("{name}@{version}")],
            t!("crates.upgrade-rationale"),
        );
        let root = self.root();
        self.run(&plan, root, |app, ended| {
            if ended == run::Ended::Done {
                app.crates_reload();
                app.crates_ask();
            }
        });
    }

    // ---- features ----------------------------------------------------------

    pub(super) fn features_pick(self: &Rc<Self>, package: &str) {
        self.features_apply(FeatureSelection {
            package: package.to_string(),
            features: Vec::new(),
            default_features: true,
        });
    }

    pub(super) fn features_toggle(self: &Rc<Self>, feature: &str) {
        let Some(mut selection) = self.state.borrow().crates.selection.clone() else {
            return;
        };
        let enabled = self
            .state
            .borrow()
            .crates
            .features
            .iter()
            .any(|f| f.name == feature && f.enabled);
        if enabled {
            // On by default and switched off: off is only expressible with
            // the defaults off and the rest of them named.
            if selection.features.iter().any(|f| f == feature) {
                selection.features.retain(|f| f != feature);
            } else if selection.default_features {
                let defaults: Vec<String> = self
                    .state
                    .borrow()
                    .crates
                    .features
                    .iter()
                    .filter(|f| f.in_default && f.name != feature)
                    .map(|f| f.name.clone())
                    .collect();
                selection.default_features = false;
                for name in defaults {
                    if !selection.features.contains(&name) {
                        selection.features.push(name);
                    }
                }
            }
        } else {
            selection.features.push(feature.to_string());
        }
        self.features_apply(selection);
    }

    fn features_defaults(self: &Rc<Self>) {
        let Some(mut selection) = self.state.borrow().crates.selection.clone() else {
            return;
        };
        selection.default_features = !selection.default_features;
        self.features_apply(selection);
    }

    /// A selection resolved against the whole graph: each feature's row and
    /// what the selection costs.
    fn features_apply(self: &Rc<Self>, selection: FeatureSelection) {
        let Some(workspace) = self.state.borrow().crates.workspace.clone() else {
            return;
        };
        self.state.borrow_mut().crates.selection = Some(selection.clone());
        self.sync_crates();
        let asked = selection.clone();
        work::spawn(
            move || {
                let rows = workspace
                    .feature_rows(&selection)
                    .map_err(|e| e.to_string());
                let impact = workspace
                    .feature_impact(&selection)
                    .map_err(|e| e.to_string());
                (rows, impact)
            },
            move |app, (rows, impact)| {
                {
                    let mut state = app.state.borrow_mut();
                    let crates = &mut state.crates;
                    let current = crates.selection.as_ref().is_some_and(|s| {
                        s.package == asked.package
                            && s.features == asked.features
                            && s.default_features == asked.default_features
                    });
                    if !current {
                        return;
                    }
                    crates.features = rows.unwrap_or_default();
                    crates.impact = impact.ok();
                }
                app.sync_crates();
            },
        );
    }

    // ---- the build directory -----------------------------------------------

    fn disk_target(root: &std::path::Path) -> (PathBuf, rusty_core::disk::Current) {
        match Workspace::load(root) {
            Ok(workspace) => (workspace.target_directory(), workspace.current()),
            Err(_) => (root.join("target"), rusty_core::disk::Current::default()),
        }
    }

    /// Measure the build directory — slow on a big one, a scan reads every
    /// dep-info file, so only the page asks.
    fn disk_scan(self: &Rc<Self>) {
        let Some(root) = self.root() else { return };
        let idle_days = {
            let mut state = self.state.borrow_mut();
            if state.crates.disk_busy {
                return;
            }
            state.crates.disk_busy = true;
            state.crates.idle_days
        };
        self.sync_crates();
        work::spawn(
            move || {
                let (target, current) = Self::disk_target(&root);
                rusty_core::disk::scan(
                    &target,
                    &root,
                    &current,
                    rusty_core::disk::ScanOptions {
                        idle_days,
                        ..rusty_core::disk::ScanOptions::default()
                    },
                )
                .report
            },
            |app, report| {
                {
                    let mut state = app.state.borrow_mut();
                    state.crates.disk_busy = false;
                    state.crates.disk = Some(report);
                }
                app.sync_crates();
            },
        );
    }

    /// Remove the stale artifacts of one tree, or of every tree. No question:
    /// the table above the button is the preview, and nothing a sweep removes
    /// is needed by the build as configured.
    fn disk_sweep(self: &Rc<Self>, tree: Option<String>) {
        let Some(root) = self.root() else { return };
        let idle_days = self.state.borrow().crates.idle_days;
        self.disk_removal(move || {
            let (target, current) = Self::disk_target(&root);
            let policy = SweepPolicy {
                tree,
                idle_days: Some(idle_days),
                ..SweepPolicy::default()
            };
            rusty_core::disk::sweep(&target, &root, &current, &policy).map_err(|e| e.to_string())
        });
    }

    /// A tree's verb: sweep it, drop its incremental caches, or remove it.
    pub(super) fn disk_tree(self: &Rc<Self>, verb: &str, index: i32) {
        let tree = {
            let state = self.state.borrow();
            usize::try_from(index)
                .ok()
                .and_then(|at| state.crates.disk.as_ref()?.trees.get(at).cloned())
        };
        let Some(tree) = tree else { return };
        match verb {
            "sweep" => self.disk_sweep(Some(tree.path)),
            "incremental" => {
                let bytes = tree
                    .groups
                    .iter()
                    .find(|g| g.kind == DiskKind::Incremental)
                    .map_or(0, |g| g.bytes);
                let separator = if tree.path.contains('\\') { "\\" } else { "/" };
                let path = format!("{}{separator}incremental", tree.path);
                if confirm(&t!("disk.incremental-confirm", size = format::bytes(bytes))) {
                    self.disk_remove(path);
                }
            }
            "remove"
                if confirm(&t!(
                    "disk.remove-confirm",
                    path = tree.path.clone(),
                    size = format::bytes(tree.bytes)
                )) =>
            {
                self.disk_remove(tree.path);
            }
            _ => {}
        }
    }

    /// One of the extras beside the trees, or one of cargo's caches,
    /// removed after asking with its size.
    pub(super) fn disk_item(self: &Rc<Self>, index: i32, cache: bool) {
        let item = {
            let state = self.state.borrow();
            let Some(report) = state.crates.disk.as_ref() else {
                return;
            };
            let list = if cache {
                &report.cargo_home
            } else {
                &report.extras
            };
            usize::try_from(index)
                .ok()
                .and_then(|at| list.get(at).cloned())
        };
        let Some(item) = item.filter(|i| i.removable) else {
            return;
        };
        if cache {
            let name = rusty_i18n::translate(&format!("disk.cache.{}", item.label))
                .unwrap_or_else(|| item.label.clone());
            if confirm(&t!(
                "disk.cache-confirm",
                name = name,
                size = format::bytes(item.bytes)
            )) {
                let label = item.label;
                self.disk_removal(move || {
                    rusty_core::disk::remove_cargo_cache(&label).map_err(|e| e.to_string())
                });
            }
        } else if confirm(&t!(
            "disk.remove-confirm",
            path = item.path.clone(),
            size = format::bytes(item.bytes)
        )) {
            self.disk_remove(item.path);
        }
    }

    fn disk_remove(self: &Rc<Self>, path: String) {
        let Some(root) = self.root() else { return };
        self.disk_removal(move || {
            let (target, _) = Self::disk_target(&root);
            rusty_core::disk::remove_tree(&target, std::path::Path::new(&path))
                .map_err(|e| e.to_string())
        });
    }

    /// A removal off the UI thread, what it did said in Output, and the
    /// directory measured again.
    fn disk_removal(
        self: &Rc<Self>,
        job: impl FnOnce() -> Result<SweepReport, String> + Send + 'static,
    ) {
        {
            let mut state = self.state.borrow_mut();
            if state.crates.disk_busy {
                return;
            }
            state.crates.disk_busy = true;
        }
        self.sync_crates();
        work::spawn(job, |app, done| {
            app.state.borrow_mut().crates.disk_busy = false;
            app.disk_said(done);
            app.disk_scan();
        });
    }

    /// What a sweep or a removal did, in Output.
    pub(super) fn disk_said(self: &Rc<Self>, done: Result<SweepReport, String>) {
        match done {
            Ok(report) => {
                self.runner.say(
                    &t!(
                        "disk.swept",
                        count = report.removed_items,
                        size = format::bytes(report.removed_bytes)
                    ),
                    run::kind::PLAIN,
                );
                for tree in &report.locked {
                    self.runner.say(
                        &t!("disk.skipped-locked", path = tree.clone()),
                        run::kind::WARNING,
                    );
                }
                for failed in &report.failed {
                    self.runner.say(failed, run::kind::ERROR);
                }
            }
            Err(why) => {
                self.runner.say(&why, run::kind::ERROR);
                self.note(2, why);
            }
        }
    }

    pub(super) fn disk_idle(self: &Rc<Self>, days: i32) {
        self.state.borrow_mut().crates.idle_days = days.clamp(1, 365) as u32;
        self.disk_scan();
    }

    pub(super) fn disk_auto(self: &Rc<Self>, on: bool) {
        let _ = rusty_embed::config::update(move |saved| saved.disk_auto_sweep = on);
        self.sync_crates();
    }

    /// After a cargo command succeeded: the stale artifacts swept, when the
    /// switch says so — opt-in, because deleting unasked is not a default
    /// even when the deletion is safe.
    pub(super) fn disk_auto_sweep(self: &Rc<Self>) {
        if !rusty_embed::config::workbench().disk_auto_sweep {
            return;
        }
        let Some(root) = self.root() else { return };
        let idle_days = self.state.borrow().crates.idle_days;
        work::spawn(
            move || {
                let firmware = rusty_embed::project::firmware_root(&root);
                let (target, current) = Self::disk_target(&firmware);
                let policy = SweepPolicy {
                    idle_days: Some(idle_days),
                    ..SweepPolicy::default()
                };
                rusty_core::disk::sweep(&target, &firmware, &current, &policy)
                    .map_err(|e| e.to_string())
            },
            |app, done| {
                if done.as_ref().is_ok_and(|r| r.removed_items == 0) {
                    return;
                }
                app.disk_said(done);
            },
        );
    }

    // ---- drawing -------------------------------------------------------------

    pub(super) fn sync_crates(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let crates = &state.crates;
        shell.set_crates_state(if state.project.is_none() {
            0
        } else if crates.report.is_some() {
            2
        } else if crates.problem.is_some() {
            3
        } else {
            1
        });
        shell.set_crates_problem(crates.problem.clone().unwrap_or_default().into());
        if let Some(report) = &crates.report {
            let vitals = &report.vitals;
            let vital = |label: String, value: usize, hint: String| CrateVital {
                label: label.into(),
                value: value.to_string().into(),
                hint: hint.into(),
            };
            shell.set_crates_title(
                format!(
                    "{} · {}",
                    report.workspace.name, report.workspace.target_platform
                )
                .into(),
            );
            shell.set_crates_vitals(ModelRc::new(VecModel::from(vec![
                vital(
                    t!("desk.crates-members"),
                    vitals.workspace_crates,
                    String::new(),
                ),
                vital(
                    t!("desk.crates-resolved"),
                    vitals.resolved_deps,
                    t!(
                        "desk.crates-resolved-hint",
                        direct = vitals.direct_deps,
                        transitive = vitals.transitive_deps
                    ),
                ),
                vital(
                    t!("desk.crates-duplicates"),
                    vitals.duplicate_groups,
                    t!(
                        "desk.crates-duplicates-hint",
                        units = vitals.duplicate_extra_units
                    ),
                ),
                vital(
                    t!("desk.crates-serial"),
                    vitals.build_scripts + vitals.proc_macros,
                    t!("features.units-hint"),
                ),
            ])));
            let dups: Vec<CrateDup> = report
                .duplicates
                .iter()
                .map(|group| CrateDup {
                    name: group.name.as_str().into(),
                    versions: group
                        .versions
                        .iter()
                        .map(|v| {
                            let by = v
                                .pulled_by
                                .iter()
                                .map(|p| {
                                    if p.is_workspace_member {
                                        format!("{} ({})", p.package, p.req)
                                    } else {
                                        format!("{} {}", p.package, p.version)
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("{} ← {by}", v.version)
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                        .into(),
                    unifiable: group.unifiable,
                })
                .collect();
            shell.set_crates_dups(ModelRc::new(VecModel::from(dups)));
            let chosen = crates.selection.as_ref().map(|s| s.package.as_str());
            let packages: Vec<FeatPackage> = report
                .members
                .iter()
                .map(|m| FeatPackage {
                    name: m.name.as_str().into(),
                    count: m.features.len() as i32,
                    chosen: Some(m.name.as_str()) == chosen,
                })
                .collect();
            shell.set_feat_packages(ModelRc::new(VecModel::from(packages)));
        } else {
            shell.set_crates_vitals(ModelRc::default());
            shell.set_crates_dups(ModelRc::default());
            shell.set_feat_packages(ModelRc::default());
        }
        shell.set_crates_asking(crates.asking);
        shell.set_crates_asked(crates.registry.is_some());
        let deps: Vec<CrateDep> = crates
            .registry
            .iter()
            .flatten()
            .map(|row| CrateDep {
                name: row.name.as_str().into(),
                current: row.current.as_str().into(),
                latest: row.latest.clone().unwrap_or_default().into(),
                note: row.note.clone().unwrap_or_default().into(),
                behind: row.latest.as_deref().is_some_and(|l| l != row.current),
            })
            .collect();
        shell.set_crates_deps(ModelRc::new(VecModel::from(deps)));

        // Features.
        let selection = crates.selection.as_ref();
        shell.set_feat_defaults(selection.is_none_or(|s| s.default_features));
        shell.set_feat_none(
            match selection {
                Some(s) if crates.features.is_empty() => {
                    t!("features.none-declared", package = s.package.clone())
                }
                _ => String::new(),
            }
            .into(),
        );
        let rows: Vec<FeatRow> = crates
            .features
            .iter()
            .map(|row| {
                let (delta, tone) = match row.marginal_crates {
                    0 => (t!("features.no-change"), 0),
                    d if d > 0 => (t!("features.crates-delta", delta = format!("+{d}")), 2),
                    d => (t!("features.crates-delta", delta = d.to_string()), 1),
                };
                FeatRow {
                    name: row.name.as_str().into(),
                    enabled: row.enabled,
                    in_default: row.in_default,
                    enables: row.enables.join(", ").into(),
                    delta: delta.into(),
                    tone,
                }
            })
            .collect();
        shell.set_feat_rows(ModelRc::new(VecModel::from(rows)));
        match &crates.impact {
            Some(impact) => {
                shell.set_feat_ready(true);
                shell.set_feat_resolved(impact.resolved_crates.to_string().into());
                shell.set_feat_baseline(
                    t!("features.baseline-hint", count = impact.baseline_crates).into(),
                );
                let signed = |n: i32| {
                    if n > 0 {
                        format!("+{n}")
                    } else {
                        n.to_string()
                    }
                };
                shell.set_feat_delta(signed(impact.delta_crates).into());
                shell.set_feat_units(signed(impact.delta_build_units).into());
                shell.set_feat_added(impact.added.join(", ").into());
                shell.set_feat_removed(impact.removed.join(", ").into());
            }
            None => shell.set_feat_ready(false),
        }

        // The build directory.
        shell.set_disk_busy(crates.disk_busy);
        shell.set_disk_idle(crates.idle_days as i32);
        shell.set_disk_auto(rusty_embed::config::workbench().disk_auto_sweep);
        shell.set_disk_idle_options(ModelRc::new(VecModel::from(
            IDLE.iter()
                .map(|days| DiskIdle {
                    days: *days as i32,
                    label: t!("disk.days", days = days).into(),
                })
                .collect::<Vec<_>>(),
        )));
        let Some(report) = &crates.disk else {
            shell.set_disk_ready(false);
            return;
        };
        shell.set_disk_ready(true);
        let low = report.volume.is_some_and(|v| {
            v.free_bytes < 5 * 1024 * 1024 * 1024 || v.free_bytes * 20 < v.total_bytes
        });
        shell.set_disk_low(low);
        shell.set_disk_volume(
            report
                .volume
                .map(|v| {
                    t!(
                        "disk.volume",
                        free = format::bytes(v.free_bytes),
                        total = format::bytes(v.total_bytes)
                    )
                })
                .unwrap_or_default()
                .into(),
        );
        shell.set_disk_dir(report.target_dir.as_str().into());
        shell.set_disk_total(
            format!(
                "{} · {}",
                t!(
                    "disk.total",
                    size = format::bytes(report.total_bytes),
                    files = report.files
                ),
                if report.shared {
                    t!("disk.shared")
                } else {
                    t!("disk.not-shared")
                }
            )
            .into(),
        );
        let stale_total: u64 = report
            .trees
            .iter()
            .flat_map(|t| t.groups.iter())
            .map(|g| g.stale_bytes)
            .sum();
        shell.set_disk_stale(
            if stale_total > 0 {
                t!("disk.stale-total", size = format::bytes(stale_total))
            } else {
                String::new()
            }
            .into(),
        );
        shell.set_disk_locked(report.trees.iter().any(|t| t.locked));
        let trees: Vec<DiskTree> = report
            .trees
            .iter()
            .enumerate()
            .map(|(index, tree)| {
                let stale: u64 = tree.groups.iter().map(|g| g.stale_bytes).sum();
                let mut by_reason: Vec<(String, u64)> = Vec::new();
                for group in &tree.groups {
                    for summary in &group.stale_by_reason {
                        match by_reason.iter_mut().find(|(r, _)| *r == summary.reason) {
                            Some((_, bytes)) => *bytes += summary.bytes,
                            None => by_reason.push((summary.reason.clone(), summary.bytes)),
                        }
                    }
                }
                by_reason.sort_by_key(|entry| std::cmp::Reverse(entry.1));
                let reasons = by_reason
                    .iter()
                    .map(|(reason, bytes)| {
                        format!(
                            "{} {}",
                            rusty_i18n::translate(&format!("disk.reason.{reason}"))
                                .unwrap_or_else(|| reason.clone()),
                            format::bytes(*bytes)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" · ");
                let incremental = tree
                    .groups
                    .iter()
                    .find(|g| g.kind == DiskKind::Incremental)
                    .map_or(0, |g| g.bytes);
                DiskTree {
                    index: index as i32,
                    label: match &tree.triple {
                        Some(triple) => format!("{triple} / {}", tree.profile),
                        None => format!("{} / {}", t!("disk.host"), tree.profile),
                    }
                    .into(),
                    size: format::bytes(tree.bytes).into(),
                    stale: if stale > 0 {
                        format::bytes(stale)
                    } else {
                        "—".into()
                    }
                    .into(),
                    has_stale: stale > 0,
                    reasons: reasons.into(),
                    locked: tree.locked,
                    incremental: if incremental > 0 {
                        t!("disk.drop-incremental", size = format::bytes(incremental))
                    } else {
                        String::new()
                    }
                    .into(),
                }
            })
            .collect();
        shell.set_disk_trees(ModelRc::new(VecModel::from(trees)));
        let item = |cache: bool| {
            move |(index, item): (usize, &rusty_core::DiskItem)| {
                let key = if cache {
                    format!("disk.cache.{}", item.label)
                } else {
                    format!("disk.extra.{}", item.label)
                };
                DiskItemRow {
                    index: index as i32,
                    name: rusty_i18n::translate(&key)
                        .unwrap_or_else(|| item.label.clone())
                        .into(),
                    size: format::bytes(item.bytes).into(),
                    removable: item.removable,
                    cache,
                }
            }
        };
        shell.set_disk_extras(ModelRc::new(VecModel::from(
            report
                .extras
                .iter()
                .enumerate()
                .map(item(false))
                .collect::<Vec<_>>(),
        )));
        shell.set_disk_caches(ModelRc::new(VecModel::from(
            report
                .cargo_home
                .iter()
                .enumerate()
                .map(item(true))
                .collect::<Vec<_>>(),
        )));
        // One shared build directory for every project: rusty does not
        // write the user's cargo config, so it says what to write and where.
        let (snippet, place) = match (&report.cargo_home_dir, report.shared) {
            (Some(home), false) => {
                let separator = if home.contains('\\') { "\\" } else { "/" };
                let dir = format!("{home}{separator}target");
                (
                    format!("[build]\ntarget-dir = \"{}\"", dir.replace('\\', "\\\\")),
                    t!(
                        "disk.share-where",
                        path = format!("{home}{separator}config.toml")
                    ),
                )
            }
            _ => (String::new(), String::new()),
        };
        shell.set_disk_share(snippet.into());
        shell.set_disk_share_where(place.into());
        shell.set_disk_debuginfo(
            if report.debuginfo_bytes > 0 {
                t!(
                    "disk.debuginfo-advice",
                    size = format::bytes(report.debuginfo_bytes)
                )
            } else {
                String::new()
            }
            .into(),
        );
        shell.set_disk_warnings(ModelRc::new(VecModel::from(
            report
                .warnings
                .iter()
                .map(|w| SharedString::from(w.as_str()))
                .collect::<Vec<_>>(),
        )));
    }
}
