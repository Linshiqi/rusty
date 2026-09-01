//! Interface language.

use leptos::prelude::*;

use crate::view::components::{Pill, Tone};

use super::*;

#[component]
pub(super) fn Language() -> impl IntoView {
    view! {
        <Field
            label="Interface language"
            help="Translations will be checked at compile time, so a missing string is a build \
                  error rather than an English word appearing in another language."
        >
            <div class="flex items-center gap-2">
                <Pill label="English" tone=Tone::Rust />
                <Pill label="简体中文" />
                <span class="text-callout text-label-3">"not wired up yet"</span>
            </div>
        </Field>
    }
}
