//! Component values as people write them on a schematic: `4k7`, `100nF`,
//! `3V3`.

/// A resistance from the way people write one: `220`, `4.7k`, `10K`, `1M`,
/// `220R`, `4k7`. Anything else is nothing, and says so — which is the
/// whole reason this returns an `Option`. A part whose value is a colour, a
/// part number or blank has no resistance the sheet can stand behind, and
/// every rule that needs one refuses rather than assuming a number.
///
/// It lives here rather than beside the colour bands it was written for
/// because the bands and the arithmetic must read a value the same way: a
/// resistor drawn as 10k and computed as nothing is two answers to one
/// question.
pub fn ohms(value: &str) -> Option<f64> {
    let text: String = compact(value)
        .chars()
        .filter(|c| *c != 'Ω' && *c != 'ω')
        .collect();
    multiplied(text.trim_end_matches(['R', 'r']), |c| match c {
        'k' | 'K' => Some(1e3),
        'M' => Some(1e6),
        'G' => Some(1e9),
        'R' | 'r' => Some(1.0),
        _ => None,
    })
}

/// A capacitance the way people write one: `100n`, `100nF`, `10u`, `10µF`,
/// `4n7`, `1p`, `2.2u`.
///
/// The same shape as [`ohms`] — the multiplier standing in for the decimal
/// point, because that is how it is written on a schematic — with the
/// suffixes a capacitor uses. Anything else is nothing and says so: a part
/// whose value is a part number has no capacitance the sheet can stand
/// behind, and a transient computed from an invented one is a settling time
/// that looks measured and is not.
pub fn farads(value: &str) -> Option<f64> {
    multiplied(compact(value).trim_end_matches(['F', 'f']), |c| match c {
        'p' | 'P' => Some(1e-12),
        'n' | 'N' => Some(1e-9),
        'u' | 'U' | 'µ' | 'μ' => Some(1e-6),
        'm' => Some(1e-3),
        _ => None,
    })
}

/// A voltage from the way people write one on a rail: `3V3`, `3.3V`,
/// `+5V`, `5`, `12`.
///
/// The `V` stands in for the decimal point exactly as `k` does in `4k7`,
/// because that is how it is written on a schematic. Anything else is
/// nothing and says so — `VCC` names a rail without saying what it is at,
/// and a solver that read it as five volts would be inventing the number
/// every answer downstream depends on.
pub fn volts(value: &str) -> Option<f64> {
    let text = compact(value);
    multiplied(text.strip_prefix('+').unwrap_or(&text), |c| {
        matches!(c, 'V' | 'v').then_some(1.0)
    })
}

/// The value with its whitespace taken out: `4.7 k` reads as `4.7k`.
fn compact(value: &str) -> String {
    value.chars().filter(|c| !c.is_whitespace()).collect()
}

/// A number with a multiplier after it (`4.7k`), in place of its decimal
/// point (`4k7`), or none at all (`4700`). The first letter `scale` knows
/// is the multiplier; anything else in the text leaves it unreadable.
fn multiplied(text: &str, scale: impl Fn(char) -> Option<f64>) -> Option<f64> {
    let Some((index, letter)) = text.char_indices().find(|(_, c)| scale(*c).is_some()) else {
        return text.parse().ok();
    };
    let (head, rest) = text.split_at(index);
    let tail = &rest[letter.len_utf8()..];
    let head: f64 = head.parse().ok()?;
    let factor = scale(letter)?;
    if tail.is_empty() {
        return Some(head * factor);
    }
    let digits: f64 = tail.parse().ok()?;
    Some((head + digits / 10f64.powi(tail.len() as i32)) * factor)
}
