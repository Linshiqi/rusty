//! The three docks and what each holds.
//!
//! Zed's model: a dock is a set of panels with one in front. Left, the
//! project's own views; bottom, what the work prints and what a run
//! brings; right, the board and the assistant, so the board stands beside
//! the code. Which are open, how big they are and which panel is in front
//! live in `workbench.toml` (`rusty_embed::WindowLayout`), with the panels
//! somebody took off a strip, which come back when they are shown.

use rusty_embed::{DockLayout, WindowLayout};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dock {
    Left,
    Bottom,
    Right,
}

impl Dock {
    pub fn parse(name: &str) -> Option<Dock> {
        match name {
            "left" => Some(Dock::Left),
            "bottom" => Some(Dock::Bottom),
            "right" => Some(Dock::Right),
            _ => None,
        }
    }

    /// The dock's panels: id, catalogue key of the name, icon.
    pub fn panels(self) -> &'static [(&'static str, &'static str, Glyph)] {
        match self {
            Dock::Left => &[
                ("project", "desk.project", Glyph::Files),
                ("search", "panel.search", Glyph::Search),
                ("git", "panel.git", Glyph::Branch),
            ],
            Dock::Bottom => &[
                ("output", "dock.tab.output", Glyph::Output),
                ("problems", "dock.tab.problems", Glyph::Warn),
                ("terminal", "dock.tab.terminal", Glyph::Monitor),
                ("plot", "dock.tab.plot", Glyph::Plot),
                ("waves", "dock.tab.waves", Glyph::Waves),
                ("draw", "dock.tab.draw", Glyph::Draw),
                ("flight", "dock.tab.flight", Glyph::Flight),
                ("signals", "dock.tab.signals", Glyph::Signals),
                ("debug", "dock.tab.debug", Glyph::Debug),
                ("registers", "dock.tab.registers", Glyph::Registers),
                ("calls", "dock.tab.calls", Glyph::Calls),
            ],
            Dock::Right => &[
                ("board", "desk.board", Glyph::Board),
                ("assistant", "panel.assistant", Glyph::Assistant),
            ],
        }
    }

    pub fn of(self, layout: &WindowLayout) -> &DockLayout {
        match self {
            Dock::Left => &layout.left,
            Dock::Bottom => &layout.bottom,
            Dock::Right => &layout.right,
        }
    }

    pub fn of_mut(self, layout: &mut WindowLayout) -> &mut DockLayout {
        match self {
            Dock::Left => &mut layout.left,
            Dock::Bottom => &mut layout.bottom,
            Dock::Right => &mut layout.right,
        }
    }

    /// The panel in front: the one remembered when it still exists and is
    /// on the strip, the first on it otherwise.
    pub fn front(self, layout: &WindowLayout) -> &'static str {
        let panels = self.panels();
        let dock = self.of(layout);
        let shown = |id: &str| !dock.hidden.iter().any(|hidden| hidden == id);
        dock.front
            .as_deref()
            .and_then(|front| panels.iter().find(|(id, ..)| *id == front && shown(id)))
            .or_else(|| panels.iter().find(|(id, ..)| shown(id)))
            .unwrap_or(&panels[0])
            .0
    }

    /// A panel's name, for a menu that names it.
    pub fn title(self, panel: &str) -> Option<&'static str> {
        self.panels()
            .iter()
            .find(|(id, ..)| *id == panel)
            .map(|(_, title, _)| *title)
    }
}

/// `panel` taken off its dock's strip, VS Code's *Hide 'Output'*; the one
/// in front of it when it was in front. Refused — `false` — for the last
/// panel `listed` on the strip, which would leave a dock with no tab to
/// open anything from.
pub fn hide(dock: &mut DockLayout, panel: &str, listed: &[&str]) -> bool {
    let Some(next) = listed.iter().find(|id| **id != panel) else {
        return false;
    };
    if !dock.hidden.iter().any(|hidden| hidden == panel) {
        dock.hidden.push(panel.to_string());
    }
    if dock.front.as_deref().is_none_or(|front| front == panel) {
        dock.front = Some((*next).to_string());
    }
    true
}

/// What Ctrl+` does, VS Code's Toggle Terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalToggle {
    /// The bottom dock opened, or turned to the terminal, with the keys in
    /// it.
    Show,
    /// The terminal is in front already and the keys are elsewhere: the
    /// keys to it.
    Focus,
    /// The terminal has the keys: the dock put away, and the keys back to
    /// the code.
    Hide,
}

/// The toggle, from whether the bottom dock is open, the panel in front of
/// it and whether the terminal has the keys. Hidden only from inside it, so
/// a key pressed in the code always ends with the keys in the terminal.
pub fn toggle_terminal(open: bool, front: &str, keys_in_terminal: bool) -> TerminalToggle {
    match (open && front == "terminal", keys_in_terminal) {
        (true, true) => TerminalToggle::Hide,
        (true, false) => TerminalToggle::Focus,
        (false, _) => TerminalToggle::Show,
    }
}

/// The icons a dock's tab can wear, by the name `ui/icons.slint` gives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Files,
    Search,
    Branch,
    Output,
    Warn,
    Monitor,
    Board,
    Assistant,
    Plot,
    Waves,
    Draw,
    Flight,
    Signals,
    Debug,
    Registers,
    Calls,
}

/// Whether a panel is on its dock's strip: the pinned ones always, the
/// ones a run or a question brings once there is something in them — or
/// while they are in front, so a panel somebody is reading does not vanish
/// under them — and none somebody took off it, until it is shown again.
pub fn listed(
    id: &str,
    has_content: impl Fn(&str) -> bool,
    front: &str,
    hidden: &[String],
) -> bool {
    if hidden.iter().any(|hidden| hidden == id) {
        return false;
    }
    match id {
        "plot" | "waves" | "draw" | "flight" | "signals" | "debug" | "registers" | "calls" => {
            has_content(id) || front == id
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panel taken off the strip leaves it, the one in front moves to
    /// the next, the last on the strip cannot go, and a dock whose first
    /// panel is hidden fronts the next rather than one with no tab.
    #[test]
    fn a_hidden_panel_leaves_the_strip_but_never_the_last() {
        let mut layout = WindowLayout::default();
        layout.left.front = Some("project".into());
        assert!(hide(
            &mut layout.left,
            "project",
            &["project", "search", "git"]
        ));
        assert_eq!(layout.left.hidden, ["project"]);
        assert_eq!(Dock::Left.front(&layout), "search");
        assert!(!listed("project", |_| true, "search", &layout.left.hidden));
        assert!(listed("git", |_| true, "search", &layout.left.hidden));
        // Somebody else in front stays in front.
        assert!(hide(&mut layout.left, "git", &["search", "git"]));
        assert_eq!(Dock::Left.front(&layout), "search");
        assert!(!hide(&mut layout.left, "search", &["search"]));
        assert_eq!(layout.left.hidden, ["project", "git"]);
        // Remembered in front but hidden: the first on the strip instead.
        layout.left.front = Some("git".into());
        assert_eq!(Dock::Left.front(&layout), "search");
    }

    #[test]
    fn a_remembered_panel_that_no_longer_exists_falls_back_to_the_first() {
        let mut layout = WindowLayout::default();
        assert_eq!(Dock::Left.front(&layout), "project");
        layout.left.front = Some("git".into());
        assert_eq!(Dock::Left.front(&layout), "git");
        layout.left.front = Some("gone".into());
        assert_eq!(Dock::Left.front(&layout), "project");
    }

    /// VS Code's Toggle Terminal: from a closed dock or another panel the
    /// terminal comes forward with the keys; in front without them it takes
    /// them; with them, the dock goes.
    #[test]
    fn ctrl_backquote_shows_focuses_or_hides_the_terminal() {
        use TerminalToggle::*;
        assert_eq!(toggle_terminal(false, "terminal", false), Show);
        assert_eq!(toggle_terminal(false, "output", false), Show);
        assert_eq!(
            toggle_terminal(false, "terminal", true),
            Show,
            "a closed dock's terminal has no keys to give back"
        );
        assert_eq!(toggle_terminal(true, "output", false), Show);
        assert_eq!(toggle_terminal(true, "terminal", false), Focus);
        assert_eq!(toggle_terminal(true, "terminal", true), Hide);
    }
}
