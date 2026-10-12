//! The solver's numbers as the text beside a part.
//!
//! Pure, and beside the component for the reason `geometry` and `edit` are:
//! whether `0.004079928007412843` reads as `4.08 mA` is arithmetic with a
//! right answer, and arithmetic in a `view!` macro is arithmetic nothing
//! checks.
//!
//! **Engineering notation, not scientific.** A panel is read by somebody
//! holding the part, and `4.08 mA` is the number on the multimeter beside
//! them; `4.0799e-3 A` is the same fact in a form they have to convert. So
//! the exponent moves in threes and carries the prefix an electronics
//! catalogue uses — and `µ` rather than `u`, because the sheet is not a
//! plain-text netlist.

/// Three, which is what a panel this size can show without the column
/// growing and what a meter on that range gives anyway. The solver's answer
/// is exact to far more, and `Solved` is where a caller goes for it.
const FIGURES: usize = 3;

/// The prefixes, from smallest to largest, and the power of ten each names.
/// Picoamps at one end because a reverse-biased junction really does sit
/// there; giga at the other because a pull-up to nothing is megohms and a
/// gate leakage resistance is more.
const PREFIXES: [(&str, i32); 9] = [
    ("p", -12),
    ("n", -9),
    ("µ", -6),
    ("m", -3),
    ("", 0),
    ("k", 3),
    ("M", 6),
    ("G", 9),
    ("T", 12),
];

/// A volt figure: `3.30 V`, `-250 mV`, `0 V`.
pub fn volts(value: f64) -> String {
    engineering(value, "V")
}

/// An amp figure: `4.08 mA`, `12.3 µA`, `0 A`.
pub fn amps(value: f64) -> String {
    engineering(value, "A")
}

/// A watt figure: `13.5 mW`.
pub fn watts(value: f64) -> String {
    engineering(value, "W")
}

/// `value` in engineering notation with `unit` after it.
///
/// **Exact zero is `0`, with no prefix.** Every other rule here is about
/// choosing a prefix from the magnitude, and zero has none — `0.00 pV` is
/// arithmetically defensible and reads as a measurement that failed.
/// Anything not finite is a dash: a solve that produced one is a bug, and
/// printing `inf V` beside a resistor would send somebody to look at the
/// resistor.
pub fn engineering(value: f64, unit: &str) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    if value == 0.0 {
        return format!("0 {unit}");
    }
    let magnitude = value.abs().log10().floor() as i32;
    // Round *towards* the prefix below: 999.6 µA is 1.00 mA and not
    // 1000 µA, and it is the rounding that decides which, so the decade is
    // taken from the number as it will be shown.
    let decade = {
        let group = magnitude.div_euclid(3) * 3;
        let shown = value.abs() / 10f64.powi(group);
        // `999.6` at three figures is `1000`, which belongs a decade up.
        if round_to(shown, FIGURES) >= 1000.0 {
            group + 3
        } else {
            group
        }
    };
    let (prefix, power) = PREFIXES
        .iter()
        .copied()
        .min_by_key(|(_, power)| (power - decade).abs())
        // Past the ends of the table, stay at the end rather than inventing
        // a prefix — `0.001 pA` is honest and `fA` is a claim about
        // precision this has no business making.
        .unwrap_or(("", 0));
    let shown = value / 10f64.powi(power);
    format!("{} {prefix}{unit}", significant(shown, FIGURES))
}

/// `value` to `figures` significant figures, with the trailing zeros a
/// measurement carries and without the ones an integer does not: `3.30`,
/// `12.3`, `250`, `1.00`.
fn significant(value: f64, figures: usize) -> String {
    let rounded = round_to(value, figures);
    // How many digits sit left of the point decides how many go right of
    // it, which is what makes the *figures* constant rather than the
    // decimals.
    let left = if rounded == 0.0 {
        1
    } else {
        rounded.abs().log10().floor() as i32 + 1
    };
    let places = (figures as i32 - left).max(0) as usize;
    format!("{rounded:.places$}")
}

/// `value` rounded to `figures` significant figures.
fn round_to(value: f64, figures: usize) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let scale = 10f64.powi(figures as i32 - 1 - value.abs().log10().floor() as i32);
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_is_the_number_a_meter_would_show() {
        assert_eq!(volts(3.3), "3.30 V");
        assert_eq!(volts(1.0999999999), "1.10 V");
        assert_eq!(volts(0.25), "250 mV");
        assert_eq!(volts(-0.25), "-250 mV");
        assert_eq!(amps(0.004079928007412843), "4.08 mA");
        assert_eq!(amps(1.23e-5), "12.3 µA");
        assert_eq!(watts(0.0135), "13.5 mW");
        // The prefixes above unity are reached by a rail current and by
        // anything else that borrows `engineering` later.
        assert_eq!(engineering(4700.0, "Ω"), "4.70 kΩ");
        assert_eq!(engineering(1e6, "Ω"), "1.00 MΩ");
    }

    /// Zero is zero. A prefix chosen from `log10(0)` is negative infinity,
    /// and the reading it produces looks like an instrument fault.
    #[test]
    fn nothing_reads_as_nothing_and_not_as_a_very_small_something() {
        assert_eq!(volts(0.0), "0 V");
        assert_eq!(amps(0.0), "0 A");
        assert_eq!(amps(-0.0), "0 A");
    }

    /// A solve that produced one of these is a bug somewhere else, and a
    /// panel printing `inf V` beside a resistor sends somebody to check
    /// their wiring.
    #[test]
    fn what_is_not_a_number_is_not_printed_as_one() {
        assert_eq!(volts(f64::NAN), "—");
        assert_eq!(volts(f64::INFINITY), "—");
        assert_eq!(amps(f64::NEG_INFINITY), "—");
    }

    /// The rounding decides the decade, not the other way round: a figure
    /// that rounds up out of its own range moves to the next prefix rather
    /// than being shown as `1000` of the one below.
    #[test]
    fn rounding_up_out_of_a_decade_takes_the_prefix_with_it() {
        assert_eq!(amps(999.6e-6), "1.00 mA");
        assert_eq!(volts(0.9996), "1.00 V");
        assert_eq!(volts(0.9994), "999 mV");
    }

    /// Three figures throughout, so a column of them lines up and a
    /// trailing zero is a measurement rather than a rounding artefact.
    #[test]
    fn the_figures_stay_significant_rather_than_decimal() {
        assert_eq!(volts(5.0), "5.00 V");
        assert_eq!(volts(12.0), "12.0 V");
        assert_eq!(volts(120.0), "120 V");
        // Past the table's ends nothing is invented: femto is not offered,
        // and the number simply gets smaller.
        assert_eq!(amps(1e-15), "0.00100 pA");
    }
}
