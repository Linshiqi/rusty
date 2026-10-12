//! What every chart in the lab draws with: points thinned to what a column
//! of pixels can show, a range that always has height, and the path.
//!
//! Pure and tested, like the Plot panel's `band` it follows: a chart whose
//! arithmetic lived in its view got it wrong where nobody could see.

/// One colour per lane, cycled — the Plot panel's inks, so a telemetry
/// channel is the same colour in both.
pub(super) const INK: [&str; 8] = [
    "#e0a838", "#5aa9e6", "#7bc47f", "#e06c75", "#b48ead", "#56b6c2", "#d19a66", "#98c379",
];

/// The colour of what was played: the generator's own, as drawn on the
/// sheet.
pub(super) const PLAYED: &str = "#5fd0c8";

/// Points thinned to at most two a column — the lowest and the highest that
/// fall in it, in the order they came — so ten thousand samples are drawn
/// as the shape they have rather than as ten thousand segments, and a spike
/// one sample wide still reaches the height it had.
///
/// `points` are `(x, y)` with `x` from 0 to 1 across the chart.
pub(super) fn thin(points: &[(f64, f64)], columns: usize) -> Vec<(f64, f64)> {
    if columns == 0 || points.len() <= 2 * columns {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(2 * columns);
    let mut column = None;
    let (mut low, mut high) = ((0.0, f64::MAX), (0.0, f64::MIN));
    let flush = |out: &mut Vec<(f64, f64)>, low: (f64, f64), high: (f64, f64)| {
        if low.1 == f64::MAX {
            return;
        }
        if low.0 <= high.0 {
            out.push(low);
            if high != low {
                out.push(high);
            }
        } else {
            out.push(high);
            out.push(low);
        }
    };
    for &(x, y) in points {
        let at = ((x.clamp(0.0, 1.0) * columns as f64) as usize).min(columns - 1);
        if column != Some(at) {
            flush(&mut out, low, high);
            column = Some(at);
            low = (x, f64::MAX);
            high = (x, f64::MIN);
        }
        if y < low.1 {
            low = (x, y);
        }
        if y > high.1 {
            high = (x, y);
        }
    }
    flush(&mut out, low, high);
    out
}

/// The range a lane is drawn against. It always has height, so a constant
/// sits in the middle rather than on the floor — "not changing" is a
/// different claim from "at its minimum" — and a small wobble on a large
/// value is drawn small: the floor is five percent of the lane's own
/// magnitude, the Plot panel's rule for the same reason.
pub(super) fn band(values: impl Iterator<Item = f64>) -> Option<(f64, f64)> {
    let (mut low, mut high) = (f64::MAX, f64::MIN);
    for value in values.filter(|v| v.is_finite()) {
        low = low.min(value);
        high = high.max(value);
    }
    if low > high {
        return None;
    }
    let middle = (low + high) / 2.0;
    let swing = high - low;
    let floor = (middle.abs().max(swing) * 0.05).max(1e-12);
    let span = swing.max(floor);
    Some((middle - span / 2.0, middle + span / 2.0))
}

/// The SVG path of `points` in a box `width` by `height`, `y` against
/// `range` and upward, a hair inside the edges so a line at the top of its
/// range is not cut in half. A value that is not a number breaks the line
/// rather than drawing to nowhere.
pub(super) fn path(points: &[(f64, f64)], range: (f64, f64), width: f64, height: f64) -> String {
    let (low, high) = range;
    let span = (high - low).max(1e-12);
    let mut out = String::new();
    let mut pen_down = false;
    for &(x, y) in points {
        if !(x.is_finite() && y.is_finite()) {
            pen_down = false;
            continue;
        }
        let px = x * width;
        let py = height - 2.0 - (y - low) / span * (height - 4.0);
        out.push_str(if pen_down { " L" } else { " M" });
        out.push_str(&format!("{px:.1} {py:.1}"));
        pen_down = true;
    }
    out.trim_start().to_string()
}

/// A number for an axis: three significant figures and no exponent a
/// person has to decode.
pub(super) fn tick(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".to_string();
    }
    let digits = (2 - value.abs().log10().floor() as i32).clamp(0, 6) as usize;
    let text = format!("{value:.digits$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thinned, a long record keeps every column's extremes — the spike one
    /// sample wide is still there — and a short one is left alone.
    #[test]
    fn thinning_keeps_what_a_column_can_show() {
        let points: Vec<(f64, f64)> = (0..10_000)
            .map(|n| {
                let x = f64::from(n) / 9_999.0;
                (x, if n == 5_003 { 50.0 } else { (x * 40.0).sin() })
            })
            .collect();
        let thinned = thin(&points, 200);
        assert!(thinned.len() <= 400, "{}", thinned.len());
        assert!(
            thinned.iter().any(|(_, y)| *y == 50.0),
            "the spike survives"
        );
        let low = thinned.iter().map(|(_, y)| *y).fold(f64::MAX, f64::min);
        assert!(low < -0.99, "and so does the trough: {low}");
        // In order along the chart.
        assert!(thinned.windows(2).all(|w| w[0].0 <= w[1].0));

        let few = [(0.0, 1.0), (0.5, 2.0), (1.0, 3.0)];
        assert_eq!(thin(&few, 200), few);
    }

    /// A constant is drawn in the middle of a band with height, and a
    /// ripple on a large value gets the five-percent floor.
    #[test]
    fn a_band_always_has_height() {
        let (low, high) = band([2.0, 2.0, 2.0].into_iter()).unwrap();
        assert!(low < 2.0 && high > 2.0);
        // Five percent of its middle, 100.25.
        let (low, high) = band([100.0, 100.5].into_iter()).unwrap();
        assert!((high - low - 5.0125).abs() < 1e-9, "{low}..{high}");
        let (low, high) = band([-1.0, 1.0].into_iter()).unwrap();
        assert_eq!((low, high), (-1.0, 1.0));
        assert_eq!(band(std::iter::empty()), None);
        assert_eq!(band([f64::NAN].into_iter()), None);
    }

    #[test]
    fn a_path_breaks_where_a_value_is_not_a_number() {
        let points = [(0.0, 0.0), (0.5, f64::NAN), (1.0, 1.0)];
        let drawn = path(&points, (0.0, 1.0), 100.0, 20.0);
        assert_eq!(drawn.matches('M').count(), 2, "{drawn}");
        assert!(!drawn.contains('L'), "{drawn}");
    }

    #[test]
    fn a_tick_is_three_figures_and_no_exponent() {
        assert_eq!(tick(1234.6), "1235");
        assert_eq!(tick(12.345), "12.3");
        assert_eq!(tick(0.012345), "0.0123");
        assert_eq!(tick(-2.5), "-2.5");
        assert_eq!(tick(0.0), "0");
        assert_eq!(tick(50.0), "50");
    }
}
