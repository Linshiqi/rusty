//! Starting a new project.

use serde::{Deserialize, Serialize};

use super::Runtime;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WizardChoice {
    pub chip: String,
    pub runtime: Runtime,
    /// Crate name for the new project — and, in the workspace layout, the
    /// name of the root directory and the prefix of the `-core` crate.
    pub name: String,
    /// Generator option ids, e.g. `embassy`, `wifi`, `alloc`.
    #[serde(default)]
    pub options: Vec<String>,
    /// One crate, or the split this workbench is built around.
    #[serde(default)]
    pub layout: WizardLayout,
}

/// The shape of a new project.
///
/// `Single` is what the generator makes: one crate that is the firmware.
/// `Workspace` is the standard embedded layout — host-testable crates as
/// workspace members and the bare-metal crate *excluded*, so `cargo test` at
/// the root runs on this machine and never tries to build `no_std` for the
/// host — which is also the layout `project::firmware_root` was written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WizardLayout {
    #[default]
    Single,
    Workspace,
}

/// A generator option, with what turning it on costs.
///
/// A model type rather than a DTO in the Tauri layer: the frontend renders
/// these, and rule 1 is that it `use`s model types directly. A struct declared
/// beside the command would have to be mirrored by hand in the frontend, which
/// is the drift the shared types exist to make impossible.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WizardOption {
    /// What `esp-generate -o` expects.
    pub id: String,
    pub label: String,
    /// What it commits the project to, in the user's terms.
    pub detail: String,
    /// Options this one cannot work without.
    ///
    /// `esp-generate` enforces these and rejects the entire run when they are
    /// missing, so the wizard needs them to avoid offering a combination that
    /// cannot succeed.
    #[serde(default)]
    pub requires: Vec<String>,
}

/// What one choice in the wizard commits the user to.
///
/// The reason the wizard exists. A list of chip names tells a beginner nothing
/// about the fact that half of them require downloading a forked compiler.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Explanation {
    pub topic: String,
    pub detail: String,
    /// A concrete follow-on — a command to run, a target that gets used.
    pub consequence: Option<String>,
}

/// Why cargo would refuse `name` as a crate name, if it would.
///
/// Here in the model rather than beside the generator, so the *field* can say
/// it while it is being typed — the Git panel's `ref_name_problem` rule,
/// applied to the one name the generator never sees. It used to be checked
/// only on the backend, on every keystroke, and each refusal arrived as a red
/// banner over the workbench: typing `flyegg` through a Chinese IME put two
/// of them there, about `f'l` and `f'l'y`, which are the input method's own
/// pinyin segmentation and not anything the user had typed.
///
/// The workspace layout is what makes this matter: the project's name becomes
/// `<name>-core`, a package cargo has to accept, and the generator is asked
/// for a crate called `firmware` whatever the project is called — so nothing
/// downstream would refuse the name until cargo did, minutes later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrateNameProblem {
    Empty,
    /// Anything that is not a letter, a digit, `-` or `_`.
    Character(char),
    /// A first character that is not a letter or a digit.
    Start(char),
}

/// `None` when cargo would take `name` as a package name.
pub fn crate_name_problem(name: &str) -> Option<CrateNameProblem> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Some(CrateNameProblem::Empty);
    };
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '-' || *c == '_'))
    {
        return Some(CrateNameProblem::Character(bad));
    }
    if !first.is_ascii_alphanumeric() {
        return Some(CrateNameProblem::Start(first));
    }
    None
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn a_name_cargo_takes_has_no_problem() {
        for good in ["blinky", "cf-drone_rs2", "a", "2fast"] {
            assert_eq!(crate_name_problem(good), None, "{good}");
        }
    }

    /// The character is named, because "not a name cargo accepts" over a
    /// field with twelve characters in it does not say which one.
    #[test]
    fn a_name_cargo_refuses_names_the_character() {
        assert_eq!(crate_name_problem(""), Some(CrateNameProblem::Empty));
        assert_eq!(
            crate_name_problem("my project"),
            Some(CrateNameProblem::Character(' '))
        );
        assert_eq!(
            crate_name_problem("\u{9a71}\u{52a8}"),
            Some(CrateNameProblem::Character('\u{9a71}'))
        );
        assert_eq!(
            crate_name_problem("-lead"),
            Some(CrateNameProblem::Start('-'))
        );
        // An input method's own pinyin segmentation, which is what reached
        // the backend on every keystroke and bannered.
        assert_eq!(
            crate_name_problem("f'l'y"),
            Some(CrateNameProblem::Character('\''))
        );
    }
}
