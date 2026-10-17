//! The code's font, chosen on the machine rather than named in the theme.
//!
//! `Theme.mono` named one family, `Cascadia Mono`, which Windows ships and
//! macOS and Linux do not. Slint draws an unknown family in its fallback
//! font without a word, and the fallback is proportional — so on a Mac the
//! editor measured a run of `M` in a sans-serif, took that for every
//! character's width (`editor/metrics.rs` assumes a monospace), and drew
//! each token at a column the glyphs never reached: a line of code with a
//! gap after every word. The layout was right; the font it was told it had
//! was not the one it had.
//!
//! So the family is tried, not trusted. Each candidate is set on the window
//! and a run of the widest letter and a run of the narrowest are measured
//! through the editor's own probe: equal, the font is monospaced and that is
//! the one; unequal, the platform drew something else and the next is tried.
//! Measured, because "this platform ships this font" is a guess somebody's
//! stripped-down install breaks, and refuse-rather-than-guess is the rule.

use std::sync::OnceLock;

use slint::ComponentHandle;

use crate::{AppWindow, Theme, TornWindow};

/// What the main window settled on, for the windows made after it.
static CHOSEN: OnceLock<String> = OnceLock::new();

/// The families to try, in order: first the one the window was designed
/// in, then the ones the platform has shipped for years. Every list ends in
/// a family that has been on the platform for as long as the platform.
pub fn candidates() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["Menlo", "SF Mono", "Monaco", "Courier New"]
    } else if cfg!(windows) {
        &["Cascadia Mono", "Consolas", "Lucida Console", "Courier New"]
    } else {
        &[
            "DejaVu Sans Mono",
            "Liberation Mono",
            "Noto Sans Mono",
            "Ubuntu Mono",
            "FreeMono",
        ]
    }
}

/// Whether a font measured monospaced: a run of `M` and a run of `i` the
/// same width, to within the rounding of one run. In a proportional font the
/// two differ by a factor of three or so; nothing lands near the line.
pub fn is_monospace(wide: f32, narrow: f32) -> bool {
    wide > 0.0 && (wide - narrow).abs() <= 1.0
}

/// Set the first candidate this machine draws monospaced on `window`, and
/// remember it for the windows torn off later. With none — a machine
/// stripped of every family on the list — the first stays, and stderr says
/// what the editor's columns will look like and why.
pub fn choose(window: &AppWindow) -> String {
    const RUN: usize = 32;
    let theme = window.global::<Theme>();
    let measure = |text: &str| {
        window.set_probe_text(text.into());
        window.get_probe_width()
    };
    let mut chosen = None;
    for family in candidates() {
        theme.set_mono((*family).into());
        let wide = measure(&"M".repeat(RUN));
        let narrow = measure(&"i".repeat(RUN));
        if is_monospace(wide, narrow) {
            chosen = Some(*family);
            break;
        }
    }
    let family = match chosen {
        Some(family) => family,
        None => {
            let first = candidates()[0];
            theme.set_mono(first.into());
            eprintln!(
                "rusty: none of {} draws monospaced here; the editor's columns will not line up",
                candidates().join(", ")
            );
            first
        }
    };
    let _ = CHOSEN.set(family.to_string());
    family.to_string()
}

/// Give a torn-off window the family the main window chose.
pub fn adopt(window: &TornWindow) {
    if let Some(family) = CHOSEN.get() {
        window.global::<Theme>().set_mono(family.as_str().into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Equal runs are a monospace; a sans-serif's `M` against its `i` is
    /// not, and nothing measured is not either.
    #[test]
    fn a_font_is_monospaced_when_its_widest_and_narrowest_runs_agree() {
        assert!(is_monospace(240.0, 240.0));
        assert!(is_monospace(240.0, 239.5));
        assert!(!is_monospace(300.0, 96.0));
        assert!(!is_monospace(0.0, 0.0));
    }

    /// Every platform has somewhere to fall to, and the first candidate is
    /// the family the window was designed in on Windows.
    #[test]
    fn every_platform_has_candidates_and_windows_keeps_its_design_font() {
        assert!(candidates().len() >= 2);
        if cfg!(windows) {
            assert_eq!(candidates()[0], "Cascadia Mono");
        }
    }
}
