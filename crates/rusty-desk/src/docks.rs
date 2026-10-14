//! The three docks and what each holds.
//!
//! Zed's model: a dock is a set of panels with one in front. Left, the
//! project's own views; bottom, what the work prints and what a run
//! brings; right, the board and the assistant, so the board stands beside
//! the code. Which are open, how big they are and which panel is in front
//! live in `workbench.toml` (`rusty_embed::WindowLayout`).

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

    /// The panel in front: the one remembered when it still exists, the
    /// first otherwise.
    pub fn front(self, layout: &WindowLayout) -> &'static str {
        let panels = self.panels();
        self.of(layout)
            .front
            .as_deref()
            .and_then(|front| panels.iter().find(|(id, ..)| *id == front))
            .unwrap_or(&panels[0])
            .0
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
}

/// Whether a panel is on its dock's strip: the pinned ones always, the
/// ones a run brings once there is something in them — or while they are
/// in front, so a panel somebody is reading does not vanish under them.
pub fn listed(id: &str, has_content: impl Fn(&str) -> bool, front: &str) -> bool {
    match id {
        "plot" | "waves" | "draw" => has_content(id) || front == id,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remembered_panel_that_no_longer_exists_falls_back_to_the_first() {
        let mut layout = WindowLayout::default();
        assert_eq!(Dock::Left.front(&layout), "project");
        layout.left.front = Some("git".into());
        assert_eq!(Dock::Left.front(&layout), "git");
        layout.left.front = Some("gone".into());
        assert_eq!(Dock::Left.front(&layout), "project");
    }
}
