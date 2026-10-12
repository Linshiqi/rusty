//! How brightly a lamp is drawn.
//!
//! A lamp's light is its current: an LED gives out close to in proportion
//! to what flows through it, across the few milliamps a sheet runs at. And
//! the eye sees the average of a lamp under PWM — above a few hundred hertz
//! nobody sees the pulses, only a dimmer lamp. So a lamp is drawn from the
//! average current `rusty_embed::period` reads for it, and both its resistor
//! and its duty decide how bright it is.
//!
//! The screen's own curve is undone on the way. A pixel's value is its
//! light to the power 1/2.2, so a lamp drawn at a value in proportion to its
//! current would look far brighter than half as bright at half the current.
//! Drawn at the current's 1/2.2 power instead, the light leaving the screen
//! is in proportion to the light the lamp would give, and the eye then does
//! to it what it would do to the lamp — which is why a duty ramped in a
//! straight line looks here as it does on the desk: brightening fast, then
//! hardly at all.

use super::geometry::rgb_color;

/// The current a lamp is drawn at full brightness for: the one its `vf` is
/// quoted at, where indicator LEDs are specified (`circuit`'s `LAMP_AT`).
/// Above it a lamp is drawn no brighter, because a screen has no brighter
/// to give.
pub(super) const FULL_AMPS: f64 = 0.010;

/// The screen's curve, undone.
const GAMMA: f64 = 2.2;

/// How brightly a lamp carrying `amps` on average is drawn, from 0 to 1.
pub(super) fn of_current(amps: f64) -> f64 {
    // `>` is false for NaN as well as for a lamp run backwards.
    if amps > 0.0 {
        (amps / FULL_AMPS).min(1.0).powf(1.0 / GAMMA)
    } else {
        0.0
    }
}

/// How brightly a lamp the rules call lit for `share` of a period is drawn:
/// as though lit meant fully, which is all the rules can say about it.
pub(super) fn of_share(share: f64) -> f64 {
    of_current(share * FULL_AMPS)
}

/// An RGB lens from how much of the period each channel is lit: the colour
/// its channels mix to, and how brightly it is drawn.
///
/// The colour is the mix at the channels' proportions, with the eight
/// corners the colours a lens has always been drawn in, and the brightness
/// is the brightest channel's — so red at a tenth of its duty is a dim red
/// and not a darker, browner colour.
pub(super) fn lens(red: f64, green: f64, blue: f64) -> (String, f64) {
    let (r, g, b) = (of_share(red), of_share(green), of_share(blue));
    let brightest = r.max(g).max(b);
    if brightest <= 0.0 {
        return (rgb_color(false, false, false).to_string(), 0.0);
    }
    let (r, g, b) = (r / brightest, g / brightest, b / brightest);
    let mut mixed = [0.0_f64; 3];
    for corner in 0..8u8 {
        let (on_r, on_g, on_b) = (corner & 1 != 0, corner & 2 != 0, corner & 4 != 0);
        let weight = [(on_r, r), (on_g, g), (on_b, b)]
            .iter()
            .map(|(on, x)| if *on { *x } else { 1.0 - x })
            .product::<f64>();
        if weight > 0.0 {
            let colour = channels(rgb_color(on_r, on_g, on_b));
            for (sum, part) in mixed.iter_mut().zip(colour) {
                *sum += weight * f64::from(part);
            }
        }
    }
    (hex(mixed), brightest)
}

/// A colour `t` of the way from `dark` to `lit`: a segment of a digit lit
/// for part of the period.
pub(super) fn mix(dark: &str, lit: &str, t: f64) -> String {
    let t = t.clamp(0.0, 1.0);
    let (from, to) = (channels(dark), channels(lit));
    let mut mixed = [0.0; 3];
    for (i, sum) in mixed.iter_mut().enumerate() {
        *sum = f64::from(from[i]) + t * (f64::from(to[i]) - f64::from(from[i]));
    }
    hex(mixed)
}

/// `#rrggbb` as its three channels. Only ever handed this file's own
/// constants; anything else reads as black rather than as a panic in a view.
fn channels(colour: &str) -> [u8; 3] {
    let digits = colour.trim_start_matches('#');
    let at = |i: usize| {
        digits
            .get(i..i + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .unwrap_or(0)
    };
    [at(0), at(2), at(4)]
}

fn hex(channels: [f64; 3]) -> String {
    let [r, g, b] = channels.map(|c| c.round().clamp(0.0, 255.0) as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lamp_is_dark_at_nothing_and_full_at_its_rated_current() {
        assert_eq!(of_current(0.0), 0.0);
        assert_eq!(
            of_current(-0.001),
            0.0,
            "a lamp run backwards gives nothing"
        );
        assert_eq!(of_current(f64::NAN), 0.0);
        assert_eq!(of_current(FULL_AMPS), 1.0);
        assert_eq!(of_current(0.020), 1.0, "and no brighter past it");
    }

    /// The question the user asked: does changing the resistor change the
    /// light. The playground's lamp with 220 Ω (about 5.9 mA) against the
    /// same lamp with 1 kΩ (about 1.3 mA) has to be two visibly different
    /// lamps, not two shades of "on".
    #[test]
    fn a_larger_resistor_is_a_visibly_dimmer_lamp() {
        let (with_220, with_1k, with_10k) = (
            of_current((3.3 - 2.0) / 220.0),
            of_current((3.3 - 2.0) / 1000.0),
            of_current((3.3 - 2.0) / 10_000.0),
        );
        assert!(with_220 - with_1k > 0.3, "{with_220} against {with_1k}");
        assert!(
            with_1k > with_10k && with_10k > 0.1,
            "dim is still lit: {with_10k}"
        );
    }

    /// Drawn so the light leaving the screen is in proportion to the
    /// current: half the current is half the light, which is a good deal
    /// more than half the pixel value.
    #[test]
    fn the_light_on_the_screen_is_in_proportion_to_the_current() {
        for (a, b) in [(0.001, 0.002), (0.0005, 0.005), (0.003, 0.009)] {
            let ratio = of_current(a).powf(GAMMA) / of_current(b).powf(GAMMA);
            assert!((ratio - a / b).abs() < 1e-9, "{a} against {b}: {ratio}");
        }
        assert!(of_current(FULL_AMPS / 2.0) > 0.7);
    }

    /// Lit by the rules for a share of the period is that share of full.
    #[test]
    fn a_share_is_that_share_of_full_current() {
        assert_eq!(of_share(0.0), 0.0);
        assert_eq!(of_share(1.0), 1.0);
        assert_eq!(of_share(0.25), of_current(0.25 * FULL_AMPS));
    }

    /// At full, a lens is exactly the colours it was always drawn in, so a
    /// sheet with no PWM on it looks as it did.
    #[test]
    fn a_lens_at_full_is_the_colour_it_always_was() {
        for corner in 1..8u8 {
            let (r, g, b) = (corner & 1 != 0, corner & 2 != 0, corner & 4 != 0);
            let share = |on: bool| if on { 1.0 } else { 0.0 };
            let (colour, level) = lens(share(r), share(g), share(b));
            assert_eq!(colour, rgb_color(r, g, b), "{r} {g} {b}");
            assert_eq!(level, 1.0);
        }
        assert_eq!(lens(0.0, 0.0, 0.0).1, 0.0);
    }

    /// A channel at a tenth of its duty is the same hue, dimmer — not a
    /// brown the lens was never going to show.
    #[test]
    fn a_dim_channel_is_the_same_colour_dimmer() {
        let (bright, full) = lens(1.0, 0.0, 0.0);
        let (dim, level) = lens(0.1, 0.0, 0.0);
        assert_eq!(dim, bright);
        assert!(level < full && level > 0.0);
        // And two channels mix to what lies between them.
        let (orange, _) = lens(1.0, 0.3, 0.0);
        assert_ne!(orange, rgb_color(true, false, false));
        assert_ne!(orange, rgb_color(true, true, false));
    }

    #[test]
    fn a_mix_runs_from_dark_to_lit() {
        assert_eq!(mix("#3a2323", "#ff5c5c", 0.0), "#3a2323");
        assert_eq!(mix("#3a2323", "#ff5c5c", 1.0), "#ff5c5c");
        assert_eq!(mix("#000000", "#ffffff", 0.5), "#808080");
        assert_eq!(mix("#000000", "#ffffff", 7.0), "#ffffff");
    }
}
