//! The page half of `crate::scene`: what colour each kind of thing is and
//! the SVG element each drawn piece becomes. Shared by the two views of
//! space — the math toolbox's and the Draw tab's — so a vector is the same
//! arrow in the same colours in both.

use leptos::{html, prelude::*};

use crate::scene::{self, Ink, Svg};

pub use rusty_embed::scene::sketch::{PALETTE, nice_step};

/// The size of the element `host` names, kept current: a view of space
/// draws to its own box, which follows the panel's dividers, the dock
/// opening under it and the window — so an observer on the element, not the
/// window's resize, which the dock closing never fires. The callback is
/// forgotten rather than kept, and reads only through `try_`, as the Git
/// log's is. Call it in the component that renders `host`.
pub fn follow_size(host: NodeRef<html::Div>) -> RwSignal<(f64, f64)> {
    let size = RwSignal::new((800.0_f64, 480.0_f64));
    let observer = StoredValue::new_local(None::<web_sys::ResizeObserver>);
    Effect::new(move |_| {
        use wasm_bindgen::{JsCast, closure::Closure};
        let Some(element) = host.get() else {
            return;
        };
        let read = move |el: &web_sys::HtmlDivElement| {
            let (w, h) = (f64::from(el.client_width()), f64::from(el.client_height()));
            if w > 0.0 && h > 0.0 && size.try_get_untracked() != Some((w, h)) {
                let _ = size.try_set((w, h));
            }
        };
        read(&element);
        let measure = Closure::<dyn FnMut()>::new(move || {
            if let Some(Some(el)) = host.try_get_untracked() {
                read(&el);
            }
        });
        if let Ok(watch) = web_sys::ResizeObserver::new(measure.as_ref().unchecked_ref()) {
            watch.observe(&element);
            observer.update_value(|slot| {
                if let Some(old) = slot.replace(watch) {
                    old.disconnect();
                }
            });
        }
        measure.forget();
    });
    on_cleanup(move || {
        observer.try_update_value(|slot| {
            if let Some(watch) = slot.take() {
                watch.disconnect();
            }
        });
    });
    size
}

/// The `index`th colour, round again past the eighth.
pub fn palette(index: usize) -> &'static str {
    PALETTE[index % PALETTE.len()]
}

/// A colour for what something stands for: the theme's, through its
/// variables, except the axes and the palette, which are the same colours
/// in every theme for the reason the Git lanes are.
pub fn colour(ink: Ink) -> std::borrow::Cow<'static, str> {
    if let Ink::Rgb(r, g, b) = ink {
        return format!("#{r:02x}{g:02x}{b:02x}").into();
    }
    match ink {
        Ink::AxisX => "#e5484d",
        Ink::AxisY => "#30a46c",
        Ink::AxisZ => "#3e63dd",
        Ink::Grid => "var(--line)",
        Ink::Body => "var(--label-2)",
        Ink::Front | Ink::Result => "var(--rust)",
        Ink::Rotor | Ink::Ghost | Ink::Guide => "var(--label-3)",
        Ink::First => "var(--slate)",
        Ink::Second => "var(--amber)",
        Ink::Term => "var(--patina)",
        Ink::Row(i) => palette(usize::from(i)),
        Ink::Rgb(..) => unreachable!("answered above"),
    }
    .into()
}

/// One drawn piece as the element that puts it on the page.
pub fn svg_of(d: scene::Drawn) -> AnyView {
    let colour = colour(d.ink);
    match d.svg {
        Svg::Path(path) => view! {
            <path
                d=path
                fill="none"
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-dasharray=if d.dashed { "4 3" } else { "" }
                style=format!("stroke: {colour}; stroke-width: {}; opacity: {}", d.width, d.opacity)
            />
        }
        .into_any(),
        Svg::Polygon(points) => view! {
            <polygon
                points=points
                stroke-linejoin="round"
                style=format!(
                    "fill: {colour}; fill-opacity: {}; stroke: {colour}; stroke-width: 1; opacity: {}",
                    d.fill,
                    d.opacity
                )
            />
        }
        .into_any(),
        Svg::Text { x, y, text } => view! {
            <text
                x=x + 4.0
                y=y - 4.0
                font-size="11"
                style=format!("fill: {colour}; font-family: var(--font-mono, monospace); opacity: {}", d.opacity)
            >
                {text}
            </text>
        }
        .into_any(),
        Svg::Dot { x, y, r } => view! {
            <circle cx=x cy=y r=r style=format!("fill: {colour}; opacity: {}", d.opacity) />
        }
        .into_any(),
    }
}
