//! A new project, one decision at a time: the part, what its firmware is
//! written on, the project's shape and options, then a name and a folder.
//!
//! A wizard rather than a form, because the reader should not have to do
//! the filtering the tool exists to do: what a choice commits the project
//! to is said beside the one that is selected, not under all of them. Every
//! answer is `rusty_embed::wizard`'s — which parts can be started, what a
//! choice explains, the command that makes it — and a part the catalogue
//! cannot start a project for is not listed.
//!
//! Creating runs the part's generator through the runner, in the Output
//! dock, or writes rusty's own template; the workspace layout is written
//! around the generator's output once it has succeeded; then the project
//! opens.

use std::{path::PathBuf, rc::Rc};

use rusty_embed::{
    Chip, CommandPlan, CrateNameProblem, Language, Runtime, ToolchainRequirement, WizardChoice,
    WizardLayout, WizardOption, crate_name_problem, wizard,
};
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{App, Tab};
use crate::{Shell, WizFact, WizNote, WizRow, format, run::Ended, work};

/// The detail pane's key for the layout row, beside the generator's option
/// ids — a name none of them can be.
const LAYOUT: &str = "layout";

#[derive(Default)]
pub(super) struct Wizard {
    /// 0 the part, 1 the runtime, 2 the options, 3 the name and create.
    step: i32,
    choice: Option<WizardChoice>,
    /// The parts a project can be started for, read when the page opens.
    chips: Vec<Chip>,
    options: Vec<WizardOption>,
    query: String,
    /// What the detail pane describes on the options step.
    focus: Option<String>,
    creating: bool,
}

impl Wizard {
    fn chip(&self) -> Option<&Chip> {
        let id = &self.choice.as_ref()?.chip;
        self.chips.iter().find(|chip| &chip.id == id)
    }

    fn requirements(&self, id: &str) -> Vec<String> {
        self.options
            .iter()
            .find(|option| option.id == id)
            .map(|option| option.requires.clone())
            .unwrap_or_default()
    }
}

/// Whether a project can be started for `chip` at all: a die the catalogue
/// names without a package is one to open a project for, and picking it
/// used to end at the last step on "no generator".
fn creatable(chip: &Chip) -> bool {
    chip.generator.is_some() || chip.std_generator.is_some() || chip.c_template.is_some()
}

/// Turn an option on, and everything it cannot work without — ticked at
/// once, so what is on screen is what will be generated.
fn turn_on(choice: &mut WizardChoice, id: &str, requires: Vec<String>) {
    for option in std::iter::once(id.to_string()).chain(requires) {
        if !choice.options.contains(&option) {
            choice.options.push(option);
        }
    }
}

/// Turn an option off, and anything that needed it.
fn turn_off(choice: &mut WizardChoice, id: &str, needs: impl Fn(&str) -> Vec<String>) {
    choice
        .options
        .retain(|option| option != id && !needs(option).iter().any(|r| r == id));
}

fn runtime_label(chip: Option<&Chip>, runtime: Runtime) -> String {
    runtime.label_on(chip.and_then(|c| c.hal_label.as_deref()))
}

impl App {
    /// File > New project: the catalogue read afresh, the last choice kept.
    pub(super) fn open_wizard(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let wizard = &mut state.wizard;
            wizard.chips = rusty_embed::catalog::Catalog::load(None)
                .chips()
                .iter()
                .filter(|chip| creatable(chip))
                .cloned()
                .collect();
            wizard.options = wizard::options();
            wizard.query.clear();
            let known = wizard.chip().is_some();
            if !known && let Some(first) = wizard.chips.first() {
                wizard.choice = Some(WizardChoice {
                    chip: first.id.clone(),
                    runtime: Runtime::BareMetal,
                    name: "firmware".to_string(),
                    options: Vec::new(),
                    layout: WizardLayout::Single,
                    language: Language::Rust,
                });
            }
        }
        self.open_tab(Tab::Page("wizard"));
        self.sync_wizard();
    }

    fn amend(self: &Rc<Self>, edit: impl FnOnce(&mut super::State)) {
        edit(&mut self.state.borrow_mut());
        self.sync_wizard();
    }

    pub(super) fn wizard_go(self: &Rc<Self>, step: i32) {
        self.amend(|state| {
            state.wizard.step = step.clamp(0, 3);
            state.wizard.focus = None;
        });
    }

    pub(super) fn wizard_filter(self: &Rc<Self>, query: &str) {
        let query = query.to_string();
        self.amend(|state| state.wizard.query = query);
    }

    pub(super) fn wizard_hover(self: &Rc<Self>, id: &str) {
        let id = id.to_string();
        self.amend(|state| state.wizard.focus = Some(id));
    }

    pub(super) fn wizard_name(self: &Rc<Self>, name: &str) {
        let name = name.to_string();
        self.amend(|state| {
            if let Some(choice) = state.wizard.choice.as_mut() {
                choice.name = name;
            }
        });
    }

    pub(super) fn wizard_layout(self: &Rc<Self>, layout: &str) {
        let layout = if layout == "workspace" {
            WizardLayout::Workspace
        } else {
            WizardLayout::Single
        };
        self.amend(|state| {
            if let Some(choice) = state.wizard.choice.as_mut() {
                choice.layout = layout;
            }
            state.wizard.focus = Some(LAYOUT.to_string());
        });
    }

    /// A row of the current step picked: a part, a runtime, an option.
    pub(super) fn wizard_pick(self: &Rc<Self>, id: &str) {
        let id = id.to_string();
        self.amend(|state| {
            let wizard = &mut state.wizard;
            let step = wizard.step;
            let picked = wizard.chips.iter().find(|c| c.id == id).cloned();
            let requires = wizard.requirements(&id);
            let options = wizard.options.clone();
            let Some(choice) = wizard.choice.as_mut() else {
                return;
            };
            match step {
                0 => {
                    let Some(chip) = picked else { return };
                    choice.chip = chip.id.clone();
                    // A part rusty writes from its own template takes none
                    // of a generator's options, and bare metal is all there
                    // is; C only where the part has a C template.
                    if chip.writes_itself(Runtime::BareMetal) {
                        choice.options.clear();
                        choice.runtime = Runtime::BareMetal;
                    }
                    if chip.c_template.is_none() {
                        choice.language = Language::Rust;
                    }
                    if chip.std_target.is_none() {
                        choice.runtime = Runtime::BareMetal;
                    }
                }
                1 => match id.as_str() {
                    "c" => {
                        choice.language = Language::C;
                        choice.runtime = Runtime::BareMetal;
                        choice.layout = WizardLayout::Single;
                        choice.options.clear();
                    }
                    "std" => {
                        choice.language = Language::Rust;
                        choice.runtime = Runtime::EspIdf;
                    }
                    _ => {
                        choice.language = Language::Rust;
                        choice.runtime = Runtime::BareMetal;
                    }
                },
                2 => {
                    if choice.options.contains(&id) {
                        let needs = |option: &str| {
                            options
                                .iter()
                                .find(|o| o.id == option)
                                .map(|o| o.requires.clone())
                                .unwrap_or_default()
                        };
                        turn_off(choice, &id, needs);
                    } else {
                        turn_on(choice, &id, requires);
                    }
                    wizard.focus = Some(id);
                }
                _ => {}
            }
        });
    }

    /// The generator the plan names, installed through the Environment
    /// page's queue.
    pub(super) fn wizard_install(self: &Rc<Self>) {
        let program = {
            let state = self.state.borrow();
            state
                .wizard
                .choice
                .as_ref()
                .and_then(|choice| wizard::plan(choice).ok())
                .map(|plan| plan.program)
        };
        if let Some(program) = program {
            self.install_tools(vec![program]);
        }
    }

    /// Create: a folder to put it in, asked last, then the generator or
    /// rusty's template.
    pub(super) fn wizard_create(self: &Rc<Self>) {
        if self.wizard_ready().is_none() {
            return;
        }
        work::spawn(
            || rfd::FileDialog::new().pick_folder(),
            |app, picked| {
                if let Some(parent) = picked {
                    app.create_project_in(parent);
                }
            },
        );
    }

    /// Create the chosen project under `parent`: the folder the picker
    /// answered, or the probe's.
    pub fn create_project_in(self: &Rc<Self>, parent: PathBuf) {
        if let Some((choice, plan)) = self.wizard_ready() {
            self.create_in(parent, choice, plan);
        }
    }

    /// The choice and the command that makes it, when the name passes and
    /// the plan is not refused.
    fn wizard_ready(self: &Rc<Self>) -> Option<(WizardChoice, CommandPlan)> {
        let choice = self.state.borrow().wizard.choice.clone()?;
        if crate_name_problem(&choice.name).is_some() {
            return None;
        }
        match wizard::plan(&choice) {
            Ok(plan) => Some((choice, plan)),
            Err(error) => {
                self.note(2, error.to_string());
                None
            }
        }
    }

    fn create_in(self: &Rc<Self>, parent: PathBuf, choice: WizardChoice, plan: CommandPlan) {
        let destination = wizard::destination(&parent, &choice);
        // Refused before anything runs: a generator into a directory that
        // is there fails halfway or overwrites, and neither is something to
        // learn from a line in Output.
        if destination.exists() {
            self.note(
                2,
                t!("wizard.exists", path = destination.display().to_string()),
            );
            return;
        }
        // The workspace puts the generated crate inside the project as
        // `firmware/`, so the generator runs there and the scaffold around
        // it is written once it has succeeded.
        let workspace = choice.layout == WizardLayout::Workspace;
        if workspace && let Err(error) = std::fs::create_dir_all(&destination) {
            self.note(2, format!("{}: {error}", destination.display()));
            return;
        }
        let run_in = if workspace {
            destination.clone()
        } else {
            parent
        };
        self.amend(|state| state.wizard.creating = true);

        if wizard::writes_itself(&choice) {
            self.show_panel("bottom", "output");
            self.runner()
                .say(&format!("$ {}", plan.display), crate::run::kind::COMMAND);
            work::spawn(
                move || {
                    let written = wizard::write_itself(&run_in, &choice)?;
                    if workspace {
                        wizard::scaffold_workspace(&destination, &choice)?;
                    }
                    Ok::<_, rusty_embed::Error>((written, destination))
                },
                |app, done| match done {
                    Ok((written, destination)) => {
                        for file in written {
                            app.runner()
                                .say(&format!("  {file}"), crate::run::kind::PLAIN);
                        }
                        app.created(destination);
                    }
                    Err(error) => {
                        app.runner()
                            .say(&error.to_string(), crate::run::kind::ERROR);
                        app.amend(|state| state.wizard.creating = false);
                    }
                },
            );
            return;
        }

        let generated = if workspace {
            destination.join("firmware")
        } else {
            destination.clone()
        };
        let program = plan.program.clone();
        let started = self.run(&plan, Some(run_in), move |app, ended| {
            if ended == Ended::Done && generated.exists() {
                if workspace {
                    work::spawn(
                        move || {
                            wizard::scaffold_workspace(&destination, &choice)
                                .map(|()| destination)
                                .map_err(|error| error.to_string())
                        },
                        |app, scaffolded| match scaffolded {
                            Ok(destination) => app.created(destination),
                            Err(reason) => {
                                app.note(2, reason);
                                app.amend(|state| state.wizard.creating = false);
                            }
                        },
                    );
                } else {
                    app.created(destination);
                }
                return;
            }
            // The generator ran and refused, or could not start: its own
            // words are in Output, and they are the reason.
            if ended != Ended::Stopped {
                app.note(
                    2,
                    t!(
                        "wizard.generator-failed",
                        program = program,
                        path = destination.display().to_string()
                    ),
                );
            }
            app.amend(|state| state.wizard.creating = false);
        });
        if !started {
            self.amend(|state| state.wizard.creating = false);
        }
    }

    /// Made: the project opened on its firmware's entry point, where the
    /// next hour is spent — whichever of the places a generator or a
    /// template puts it exists, or nothing.
    fn created(self: &Rc<Self>, destination: PathBuf) {
        self.amend(|state| {
            state.wizard.creating = false;
            state.wizard.step = 0;
        });
        let main = [
            "src/main.rs",
            "src/bin/main.rs",
            "firmware/src/main.rs",
            "firmware/src/bin/main.rs",
            "src/main.c",
        ]
        .into_iter()
        .find(|file| destination.join(file).is_file())
        .map(str::to_string);
        self.open_project(destination, main);
    }

    pub(super) fn sync_wizard(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let wizard = &state.wizard;
        let Some(choice) = wizard.choice.as_ref() else {
            shell.set_wiz_rows(ModelRc::default());
            return;
        };
        let chip = wizard.chip();
        let c = choice.language == Language::C;
        let written = c || chip.is_some_and(|chip| chip.writes_itself(choice.runtime));

        let mut rows: Vec<WizRow> = Vec::new();
        let mut list_note = String::new();
        let mut title = String::new();
        let mut pills: Vec<SharedString> = Vec::new();
        let mut detail = String::new();
        let mut facts: Vec<WizFact> = Vec::new();
        let mut foot = String::new();
        match wizard.step {
            0 => {
                let needle = wizard.query.to_lowercase();
                rows = wizard
                    .chips
                    .iter()
                    .filter(|chip| {
                        needle.is_empty()
                            || chip.name.to_lowercase().contains(&needle)
                            || chip.arch.label().to_lowercase().contains(&needle)
                            || chip
                                .radios
                                .iter()
                                .any(|r| r.to_lowercase().contains(&needle))
                    })
                    .map(|chip| WizRow {
                        id: chip.id.as_str().into(),
                        title: chip.name.as_str().into(),
                        aside: chip.arch.label().into(),
                        on: chip.id == choice.chip,
                        check: false,
                    })
                    .collect();
                if let Some(chip) = chip {
                    title = chip.name.clone();
                    pills.push(chip.arch.label().into());
                    pills.push(t!("wizard.cores", count = chip.cores.to_string()).into());
                    if chip.needs_esp_toolchain() {
                        pills.push(t!("wizard.needs-espup").into());
                    }
                    // The one fact that most changes what the next hour
                    // looks like, on the part that has it.
                    detail = match chip.toolchain {
                        ToolchainRequirement::EspXtensa => t!("wizard.forked"),
                        ToolchainRequirement::NightlyBuildStd => t!("wizard.nightly"),
                        ToolchainRequirement::Stock => t!("wizard.stock"),
                    };
                    let mut fact = |label: &str, value: String| {
                        facts.push(WizFact {
                            label: rusty_i18n::lookup(label).into(),
                            value: value.into(),
                        })
                    };
                    fact("wizard.sram", format::bytes(chip.sram_bytes as u64));
                    if let Some(flash) = chip.flash_bytes {
                        fact("wizard.flash", format::bytes(flash as u64));
                    }
                    fact("wizard.target", chip.bare_metal_target.clone());
                    if !chip.radios.is_empty() {
                        fact("wizard.radios", chip.radios.join(", "));
                    }
                }
            }
            1 => {
                let mut row = |id: &str, title: String, on: bool| {
                    rows.push(WizRow {
                        id: id.into(),
                        title: title.into(),
                        on,
                        ..WizRow::default()
                    })
                };
                row(
                    "bare",
                    runtime_label(chip, Runtime::BareMetal),
                    !c && choice.runtime == Runtime::BareMetal,
                );
                // `std` only for a part with a `std` target: listing it and
                // failing at the end is the plausible wrong path.
                if chip.is_some_and(|chip| chip.std_target.is_some()) {
                    row(
                        "std",
                        runtime_label(chip, Runtime::EspIdf),
                        !c && choice.runtime == Runtime::EspIdf,
                    );
                }
                if chip.is_some_and(|chip| chip.c_template.is_some()) {
                    row("c", t!("wizard.c-label"), c);
                }
                (title, detail) = if c {
                    (t!("wizard.c-label"), t!("wizard.c-detail"))
                } else {
                    (
                        runtime_label(chip, choice.runtime),
                        match choice.runtime {
                            Runtime::BareMetal => t!("wizard.bare-metal"),
                            Runtime::EspIdf => t!("wizard.esp-idf"),
                        },
                    )
                };
            }
            2 => {
                if written {
                    list_note = if c {
                        t!("wizard.c-no-options")
                    } else {
                        t!("wizard.written-by-rusty")
                    };
                } else {
                    rows = wizard
                        .options
                        .iter()
                        .map(|option| WizRow {
                            id: option.id.as_str().into(),
                            title: option.label.as_str().into(),
                            aside: option.id.as_str().into(),
                            on: choice.options.contains(&option.id),
                            check: true,
                        })
                        .collect();
                }
                let option = wizard
                    .focus
                    .as_ref()
                    .and_then(|id| wizard.options.iter().find(|o| &o.id == id));
                if c {
                    (title, detail) = (t!("wizard.c-label"), t!("wizard.c-detail"));
                } else if let Some(option) = option {
                    title = option.label.clone();
                    detail = option.detail.clone();
                    // Said before it happens: the extra ticks would
                    // otherwise look like a bug.
                    if !option.requires.is_empty() {
                        foot = format!(
                            "{}{}{}",
                            t!("wizard.turns-on"),
                            option.requires.join(", "),
                            t!("wizard.turns-on-tail")
                        );
                    }
                } else if written || wizard.focus.as_deref() == Some(LAYOUT) {
                    (title, detail) = (t!("wizard.layout"), t!("wizard.layout-detail"));
                } else {
                    detail = t!("wizard.point-at");
                }
            }
            _ => {}
        }

        // The last step: the name, what the choices commit to, the command.
        let problem = crate_name_problem(&choice.name);
        let name_problem = match problem {
            None => String::new(),
            Some(CrateNameProblem::Empty) => t!("wizard.name-empty"),
            Some(CrateNameProblem::Character(c)) => {
                t!("wizard.name-character", char = c.to_string())
            }
            Some(CrateNameProblem::Start(c)) => t!("wizard.name-start", char = c.to_string()),
        };
        let mut summary: Vec<SharedString> = vec![
            chip.map_or(choice.chip.clone(), |c| c.name.clone()).into(),
            if c {
                t!("wizard.c-label")
            } else {
                runtime_label(chip, choice.runtime)
            }
            .into(),
        ];
        if !c {
            summary.push(
                rusty_i18n::lookup(match choice.layout {
                    WizardLayout::Single => "wizard.layout-single",
                    WizardLayout::Workspace => "wizard.layout-workspace",
                })
                .into(),
            );
        }
        summary.extend(
            choice
                .options
                .iter()
                .map(|o| SharedString::from(o.as_str())),
        );
        let notes: Vec<WizNote> = if wizard.step == 3 {
            wizard::explain(choice)
                .into_iter()
                .map(|e| WizNote {
                    topic: e.topic.into(),
                    detail: e.detail.into(),
                    consequence: e.consequence.unwrap_or_default().into(),
                })
                .collect()
        } else {
            Vec::new()
        };
        // No plan is asked for until the name passes: the field already says
        // what is wrong with it.
        let plan = problem.is_none().then(|| wizard::plan(choice));
        let (command, refusal) = match &plan {
            Some(Ok(plan)) => (plan.display.clone(), String::new()),
            Some(Err(error)) => (String::new(), error.to_string()),
            None => (String::new(), String::new()),
        };
        // The generator is checked before the button is offered: finding
        // out after choosing a folder is a step too late.
        let missing = match (&plan, state.environment.report()) {
            (Some(Ok(plan)), Some(report)) => report
                .status
                .tools
                .iter()
                .find(|tool| tool.name == plan.program && !tool.is_installed())
                .map(|tool| t!("wizard.not-installed", program = tool.name.clone()))
                .unwrap_or_default(),
            _ => String::new(),
        };

        shell.set_wiz_step(wizard.step);
        shell.set_wiz_rows(ModelRc::new(VecModel::from(rows)));
        shell.set_wiz_list_note(list_note.into());
        shell.set_wiz_layouts(wizard.step == 2 && !c);
        shell.set_wiz_layout(
            match choice.layout {
                WizardLayout::Single => "single",
                WizardLayout::Workspace => "workspace",
            }
            .into(),
        );
        shell.set_wiz_detail_title(title.into());
        shell.set_wiz_pills(ModelRc::new(VecModel::from(pills)));
        shell.set_wiz_detail(detail.into());
        shell.set_wiz_facts(ModelRc::new(VecModel::from(facts)));
        shell.set_wiz_detail_foot(foot.into());
        shell.set_wiz_name(choice.name.as_str().into());
        shell.set_wiz_name_problem(name_problem.into());
        shell.set_wiz_summary(ModelRc::new(VecModel::from(summary)));
        shell.set_wiz_notes(ModelRc::new(VecModel::from(notes)));
        shell.set_wiz_command(command.into());
        shell.set_wiz_refusal(refusal.into());
        shell.set_wiz_missing(missing.clone().into());
        shell.set_wiz_creating(wizard.creating);
        shell.set_wiz_can_create(
            !wizard.creating && matches!(plan, Some(Ok(_))) && missing.is_empty(),
        );
    }
}
