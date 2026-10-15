//! A formula drawn: TeX laid out by RaTeX — KaTeX's layout, in Rust — and
//! written as an SVG whose glyphs are outlines from KaTeX's own fonts, so
//! the picture needs no font on the machine that shows it.
//!
//! What comes back is measured in ems of the formula's own size: how wide it
//! is, how far it reaches above its baseline and how far below, which is
//! what a line of text needs to set it on its baseline. The SVG is black on
//! nothing; the page colours it with the theme's text colour as it draws it,
//! so a theme switch costs no formula.
//!
//! A formula RaTeX refuses comes back as its reason, never as a picture of
//! something else: the page then shows the source with the reason beside
//! it, which is what a wrong formula would cost a reader an hour to find.

use std::panic::{AssertUnwindSafe, catch_unwind};

use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_svg::{SvgOptions, render_to_svg};
use ratex_types::display_item::DisplayList;
use ratex_types::math_style::MathStyle;

/// SVG user units to an em. Any number would do — the page scales the
/// picture to its own size — but a large one keeps the path coordinates
/// whole enough to read.
const EM: f64 = 32.0;

/// Room around the box on every side, in ems: an italic letter's overhang
/// and an accent's ink can reach a little past the box TeX reports, and a
/// picture cut at the box would shave them.
pub const PAD: f32 = 0.1;

/// A formula, drawn.
#[derive(Clone, Debug)]
pub struct Formula {
    /// The SVG, `PAD` larger than the box on every side.
    pub svg: String,
    /// The box, in ems: its width, how far it reaches above the baseline and
    /// how far below.
    pub width: f32,
    pub height: f32,
    pub depth: f32,
}

/// Lay `tex` out — as display math when `display`, as a formula in a line of
/// text when not — and draw it.
pub fn render(tex: &str, display: bool) -> Result<Formula, String> {
    let list = display_list(tex, display)?;
    let options = SvgOptions {
        font_size: EM,
        padding: f64::from(PAD) * EM,
        stroke_width: 0.04 * EM,
        embed_glyphs: true,
        font_dir: String::new(),
    };
    let svg = catch_unwind(AssertUnwindSafe(|| render_to_svg(&list, &options)))
        .map_err(|_| "the formula could not be drawn".to_string())?;
    Ok(Formula {
        svg,
        width: list.width as f32,
        height: list.height as f32,
        depth: list.depth as f32,
    })
}

/// What RaTeX lays `tex` out as, or why it would not.
fn display_list(tex: &str, display: bool) -> Result<DisplayList, String> {
    let tex = tex.trim();
    if tex.is_empty() {
        return Err("the formula is empty".to_string());
    }
    let style = if display {
        MathStyle::Display
    } else {
        MathStyle::Text
    };
    // A layout engine meeting input it was not written for may panic; a
    // page is no reason to take the window down with it.
    catch_unwind(AssertUnwindSafe(|| {
        let nodes = ratex_parser::parser::parse(tex).map_err(|e| e.message.clone())?;
        let laid = layout(&nodes, &LayoutOptions::default().with_style(style));
        Ok(to_display_list(&laid))
    }))
    .unwrap_or_else(|_| Err("the formula could not be laid out".to_string()))
}

/// Every glyph a formula draws, and where: its character, its x and its y
/// (down from the top of the box), in ems. For the tests, which hold the
/// layout to what TeX would do with the same source.
#[cfg(test)]
fn glyphs(tex: &str, display: bool) -> Vec<(char, f64, f64, f64)> {
    let list = display_list(tex, display).expect("a formula RaTeX reads");
    list.items
        .iter()
        .filter_map(|item| match item {
            ratex_types::display_item::DisplayItem::GlyphPath {
                x,
                y,
                scale,
                char_code,
                ..
            } => Some((char::from_u32(*char_code)?, *x, *y, *scale)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The last glyph `c` drawn: the base and its scripts come last.
    fn glyph(glyphs: &[(char, f64, f64, f64)], c: char) -> (f64, f64, f64) {
        glyphs
            .iter()
            .rfind(|g| g.0 == c)
            .map(|g| (g.1, g.2, g.3))
            .unwrap_or_else(|| panic!("{c} is drawn"))
    }

    /// `v_i^2` is a superscript and a subscript on the same `v`, stacked at
    /// its right; not a superscript on the subscript, which is the reading a
    /// converter got wrong once — subtly, and in a formula for thrust.
    #[test]
    fn a_superscript_and_a_subscript_stack_on_their_base() {
        let drawn = glyphs(r"T = 2\rho A v_i^2", false);
        let (vx, vy, _) = glyph(&drawn, 'v');
        let (ix, iy, iscale) = glyph(&drawn, 'i');
        let (sx, sy, sscale) = glyph(&drawn, '2');
        // Both scripts are smaller than the base, start just right of it, and
        // the `2` is not pushed past the `i` as it would be on it.
        assert!(iscale < 1.0 && sscale < 1.0);
        assert!(ix > vx && sx > vx);
        assert!(
            (sx - ix).abs() < 0.15,
            "the 2 sits over the i, not after it: {sx} against {ix}"
        );
        // One above the baseline, one below it (y grows downwards).
        assert!(
            sy < vy && iy > vy,
            "2 up at {sy}, i down at {iy}, v at {vy}"
        );
    }

    /// `\begin{aligned}` is in the real book, and its rows are rows: the
    /// second row's `=` is below the first's, at the same x.
    #[test]
    fn aligned_rows_are_rows_with_their_equals_signs_aligned() {
        let drawn = glyphs(
            r"\begin{aligned} m_{FL} &= T + \tau_x \\ m &= T - \tau_x - \tau_y \end{aligned}",
            true,
        );
        let equals: Vec<_> = drawn.iter().filter(|g| g.0 == '=').collect();
        assert_eq!(equals.len(), 2);
        assert!((equals[0].1 - equals[1].1).abs() < 1e-6, "aligned at the &");
        assert!(equals[1].2 > equals[0].2 + 0.5, "one row under the other");
    }

    /// A control space is a space: nothing of it is drawn — the old page
    /// once printed the six characters of `&nbsp;` there.
    #[test]
    fn a_control_space_draws_nothing_but_space() {
        let drawn: String = glyphs(r"1\ \text{s}", false).iter().map(|g| g.0).collect();
        assert_eq!(drawn, "1s");
        let wide = render(r"1\ \text{s}", false).unwrap().width;
        let tight = render(r"1\text{s}", false).unwrap().width;
        assert!(
            wide > tight + 0.2,
            "the space is there: {wide} against {tight}"
        );
    }

    /// `\operatorname{Re}` is the letters R and e, upright — one candidate
    /// engine drew it as the symbol ℜ, a different thing.
    #[test]
    fn an_operator_name_is_its_letters() {
        let drawn: String = glyphs(r"\operatorname{Re} D", false)
            .iter()
            .map(|g| g.0)
            .collect();
        assert_eq!(drawn, "ReD");
    }

    /// A formula that cannot be read comes back as the reason, never as a
    /// picture of part of it.
    #[test]
    fn a_formula_that_is_not_tex_comes_back_as_its_reason() {
        for bad in [r"\frac{1}", r"x^", r"\begin{aligned} a \end{cases}", "  "] {
            let reason = render(bad, false).expect_err(bad);
            assert!(!reason.is_empty(), "{bad} says why");
        }
        assert!(render(r"\undefinedmacro x", false).is_err());
    }

    /// The box is what a line needs: a fraction reaches above and below the
    /// baseline, and the picture is the box with its padding round it.
    #[test]
    fn the_box_says_where_the_baseline_is() {
        let f = render(r"\frac{a}{b}", false).unwrap();
        assert!(f.height > 0.5 && f.depth > 0.2, "{f:?}");
        let x = render("x", false).unwrap();
        assert!(x.depth.abs() < 0.05 && x.height < f.height);
        let tall = render(r"\frac{a}{b}", true).unwrap();
        assert!(tall.height > f.height, "display style is larger");
        // The picture's size is the box's and the padding's.
        let size = |svg: &str, attr: &str| -> f64 {
            let at = svg.find(&format!("{attr}=\"")).unwrap() + attr.len() + 2;
            svg[at..]
                .split('"')
                .next()
                .unwrap()
                .trim_end_matches("pt")
                .parse()
                .unwrap()
        };
        let w = size(&f.svg, "width");
        assert!(
            (w - (f64::from(f.width) + 2.0 * f64::from(PAD)) * EM).abs() < 0.5,
            "{w}"
        );
    }

    /// The book writes words of Chinese inside `\text{}`; they are laid out,
    /// not refused.
    #[test]
    fn text_in_another_script_is_laid_out() {
        let f = render(r"\mathbf x_{\text{世界}}", false).unwrap();
        let narrow = render(r"\mathbf x_{\text{a}}", false).unwrap();
        assert!(f.width > narrow.width);
    }
}
