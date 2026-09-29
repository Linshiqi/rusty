//! What programs have drawn, and how it is looked at (`view/draw.rs`): the
//! scenes the `[rusty:draw]` lines built, and the camera over them.

use leptos::prelude::*;

use rusty_embed::draw::{Sketch, Sketchbook};

use crate::scene::{Camera, Look};

#[derive(Clone, Copy)]
pub struct Drawing {
    /// The scene being received, between its `scene` and its `end`. Not a
    /// signal: the tab draws only what is finished, and a mark arriving is
    /// no reason to wake it — a firmware drawing every loop sends hundreds.
    pub book: StoredValue<Sketchbook>,
    /// The finished scenes, one per title (`rusty_embed::draw::file`).
    pub sketches: RwSignal<Vec<Sketch>>,
    /// The title of the scene filed last: what the tab shows unless another
    /// was chosen.
    pub latest: RwSignal<Option<String>>,
    /// The scene chosen from the list; `None` follows the newest.
    pub chosen: RwSignal<Option<String>>,
    /// The mark the pointer is over in the list, drawn at full strength
    /// while the rest are dimmed.
    pub hovered: RwSignal<Option<usize>>,
    pub camera: RwSignal<Camera>,
    /// What the camera was last framed for. Here and not in the view, which
    /// is rebuilt whenever the tab is switched to.
    pub fitted: StoredValue<Option<Framed>>,
    /// Whether this run has brought the tab forward yet. Once, at its first
    /// scene: a firmware redrawing fifty times a second would otherwise take
    /// the dock back every time somebody looked at Output.
    pub fronted: StoredValue<bool>,
}

/// The scene a camera was framed for and the view it was framed in: a new
/// scene, this one grown or shrunk past a factor of two, or a view that
/// changed shape by a third is framed again; anything less keeps the zoom
/// somebody chose.
#[derive(Debug, Clone, PartialEq)]
pub struct Framed {
    pub title: String,
    /// How far the scene reached.
    pub reach: f64,
    /// The view, across and down, in pixels.
    pub size: (f64, f64),
}

impl Framed {
    /// Whether a scene and a view this far from what was framed want
    /// framing again.
    pub fn stale(&self, title: &str, reach: f64, size: (f64, f64)) -> bool {
        let moved = |now: f64, then: f64| (now - then).abs() > then * 0.33;
        self.title != title
            || reach > self.reach * 2.0
            || reach < self.reach / 2.0
            || moved(size.0, self.size.0)
            || moved(size.1, self.size.1)
    }
}

impl Drawing {
    pub fn fresh() -> Self {
        Drawing {
            book: StoredValue::new(Sketchbook::default()),
            sketches: RwSignal::new(Vec::new()),
            latest: RwSignal::new(None),
            chosen: RwSignal::new(None),
            hovered: RwSignal::new(None),
            camera: RwSignal::new(Camera::look(Look::Iso)),
            fitted: StoredValue::new(None),
            fronted: StoredValue::new(false),
        }
    }

    /// The title of the scene the tab shows: the chosen one while it is
    /// still there, else the newest.
    ///
    /// Tracked: a view or a memo reading it follows every choice and every
    /// scene filed.
    pub fn shown_title(&self) -> Option<String> {
        let chosen = self.chosen.get();
        let latest = self.latest.get();
        self.sketches.with(|sketches| {
            chosen
                .filter(|title| sketches.iter().any(|s| &s.title == title))
                .or(latest)
                .filter(|title| sketches.iter().any(|s| &s.title == title))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new scene, one that grew or shrank past a factor of two, and a
    /// view reshaped by a third — the dock dragged taller — are framed
    /// again; anything smaller keeps the zoom somebody chose.
    #[test]
    fn a_new_scene_or_a_reshaped_view_is_framed_again_and_little_else() {
        let framed = Framed {
            title: "cross".into(),
            reach: 2.0,
            size: (900.0, 240.0),
        };
        assert!(!framed.stale("cross", 2.5, (950.0, 250.0)));
        assert!(framed.stale("dot", 2.0, (900.0, 240.0)));
        assert!(framed.stale("cross", 4.5, (900.0, 240.0)));
        assert!(framed.stale("cross", 0.9, (900.0, 240.0)));
        assert!(framed.stale("cross", 2.0, (900.0, 400.0)));
    }
}
