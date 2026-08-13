//! The shared vocabulary every panel draws from.
//!
//! Contributed panels will be rendered with exactly these, which is why they
//! live in one place rather than being inlined per panel — see
//! `docs/extensibility.md`.
//!
//! A design system is allowed to be complete before its consumers are: a `Tone`
//! that exists for four of six severities, or a `Pill` nothing renders yet,
//! would otherwise be added and removed panel by panel. That licence stops at
//! this module — everywhere else, unused means delete.
#![allow(dead_code)]

use leptos::prelude::*;

use rusty_embed::Severity;

/// Semantic colour. Separate from the accent: the accent marks what is
/// interactive, these mark what is true.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Rust,
    Patina,
    Amber,
    Crimson,
    Slate,
}

impl Tone {
    pub fn from_severity(severity: Severity) -> Self {
        match severity {
            Severity::Blocking => Tone::Crimson,
            Severity::Warning => Tone::Amber,
            Severity::Info => Tone::Slate,
        }
    }

    fn text(self) -> &'static str {
        match self {
            Tone::Neutral => "text-label-2",
            Tone::Rust => "text-rust",
            Tone::Patina => "text-patina",
            Tone::Amber => "text-amber",
            Tone::Crimson => "text-crimson",
            Tone::Slate => "text-slate",
        }
    }

    fn fill(self) -> &'static str {
        match self {
            Tone::Neutral => "bg-sunken text-label-2",
            Tone::Rust => "bg-rust-fill text-rust",
            Tone::Patina => "bg-patina-fill text-patina",
            Tone::Amber => "bg-amber-fill text-amber",
            Tone::Crimson => "bg-crimson-fill text-crimson",
            Tone::Slate => "bg-slate-fill text-slate",
        }
    }

    fn dot(self) -> &'static str {
        match self {
            Tone::Neutral => "bg-label-3",
            Tone::Rust => "bg-rust",
            Tone::Patina => "bg-patina",
            Tone::Amber => "bg-amber",
            Tone::Crimson => "bg-crimson",
            Tone::Slate => "bg-slate",
        }
    }
}

/// A small filled label. macOS uses these for status, always with a tint rather
/// than a saturated fill.
#[component]
pub fn Pill(
    #[prop(into)] label: String,
    #[prop(default = Tone::Neutral)] tone: Tone,
) -> impl IntoView {
    view! {
        <span class=format!(
            "inline-flex h-[18px] items-center rounded-full px-2 text-caption font-semibold \
             tracking-wide uppercase whitespace-nowrap {}",
            tone.fill(),
        )>
            {label}
        </span>
    }
}

#[component]
pub fn Dot(tone: Tone) -> impl IntoView {
    view! { <span class=format!("size-1.5 shrink-0 rounded-full {}", tone.dot()) /> }
}

/// A section header: a small uppercase caption over a hairline. The sidebar and
/// the panels share it so the rhythm is the same everywhere.
#[component]
pub fn SectionLabel(#[prop(into)] label: String) -> impl IntoView {
    view! {
        <div class="px-4 pt-4 pb-1.5 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
            {label}
        </div>
    }
}

/// A readout: a number with a caption above and a hint below.
///
/// Rendered as cells of one hairline-divided grid rather than as separate
/// cards. A card array is what a generic admin dashboard looks like; an
/// instrument cluster is what a tool looks like.
#[component]
pub fn Readout(
    #[prop(into)] label: String,
    #[prop(into)] value: String,
    #[prop(optional, into)] unit: Option<String>,
    #[prop(optional, into)] hint: Option<String>,
    #[prop(default = Tone::Neutral)] tone: Tone,
) -> impl IntoView {
    view! {
        <div class="min-w-0 border-r border-line px-4 py-3 last:border-r-0">
            <div class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                {label}
            </div>
            <div class=format!(
                "tnum mt-1.5 flex items-baseline gap-1 font-mono text-[22px] leading-none \
                 font-semibold tracking-tight {}",
                tone.text(),
            )>
                {value}
                {unit.map(|u| view! { <span class="text-callout font-medium text-label-3">{u}</span> })}
            </div>
            {hint.map(|h| view! { <div class="mt-1.5 text-footnote text-label-3">{h}</div> })}
        </div>
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Normal,
    Primary,
    Quiet,
}

#[component]
pub fn Button(
    #[prop(into)] label: String,
    #[prop(default = ButtonKind::Normal)] kind: ButtonKind,
    #[prop(optional)] on_click: Option<Callback<()>>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] title: Option<String>,
) -> impl IntoView {
    let look = match kind {
        ButtonKind::Primary => "bg-rust text-white hover:brightness-110 active:brightness-95",
        ButtonKind::Normal => {
            "bg-raised text-label ring-1 ring-line hover:ring-line-strong active:bg-sunken"
        }
        ButtonKind::Quiet => "text-label-2 hover:bg-sunken hover:text-label",
    };
    view! {
        <button
            type="button"
            title=title
            disabled=move || disabled.get()
            on:click=move |_| {
                if let Some(cb) = on_click {
                    cb.run(());
                }
            }
            class=format!(
                "inline-flex h-[26px] items-center gap-1.5 rounded-[6px] px-2.5 text-callout \
                 font-medium whitespace-nowrap transition-[filter,background-color,box-shadow] \
                 duration-100 disabled:pointer-events-none disabled:opacity-40 {look}",
            )
        >
            {label}
        </button>
    }
}

/// A problem, stated the way the backend stated it: what is wrong, why it
/// matters, and the command that fixes it.
#[component]
pub fn ProblemRow(problem: rusty_embed::Problem) -> impl IntoView {
    let tone = Tone::from_severity(problem.severity);
    let fix = problem.fix_command.clone();

    view! {
        <div class="flex gap-2.5 border-b border-line px-4 py-3 last:border-b-0">
            <div class="mt-[5px]">
                <Dot tone=tone />
            </div>
            <div class="min-w-0 flex-1 select-text">
                <div class="text-body font-medium">{problem.title}</div>
                <p class="mt-0.5 max-w-[72ch] text-callout leading-relaxed text-label-2">
                    {problem.detail}
                </p>
                {fix.map(|command| view! { <div class="mt-2"><CommandLine command=command /></div> })}
            </div>
        </div>
    }
}

/// Put text on the clipboard. A refusal (no permission, no clipboard) is
/// silent: nothing was destroyed, and a banner for a failed copy is noise.
pub fn copy_to_clipboard(text: &str) {
    if let Some(window) = web_sys::window() {
        let _ = window.navigator().clipboard().write_text(text);
    }
}

/// A shell command, shown verbatim with one click to copy it.
///
/// Embedded work happens in a terminal as much as in a window, so every command
/// rusty would run — or wants the user to run — is shown rather than hidden
/// behind a button, and is one click from the clipboard. A command that has to
/// be retyped gets retyped wrong.
#[component]
pub fn CommandLine(#[prop(into)] command: String) -> impl IntoView {
    let copied = RwSignal::new(false);
    let to_copy = command.clone();

    let copy = move |_| {
        copy_to_clipboard(&to_copy);
        copied.set(true);
        // No timer to reset it: the confirmation is for the click that just
        // happened, and it goes away with the next render of this panel.
    };

    view! {
        <div class="inline-flex max-w-full items-center gap-2 rounded-[6px] bg-sunken py-1 pr-1 pl-2">
            <span class="shrink-0 font-mono text-footnote text-label-3">"$"</span>
            <code class="min-w-0 flex-1 truncate font-mono text-footnote select-text">{command}</code>
            <button
                type="button"
                title="Copy"
                on:click=copy
                class="shrink-0 rounded-[4px] px-1.5 py-0.5 text-footnote text-label-3 transition-colors hover:bg-raised hover:text-label"
            >
                {move || if copied.get() { "copied" } else { "copy" }}
            </button>
        </div>
    }
}

/// Nothing to show yet, with the one action that changes that.
#[component]
pub fn Empty(
    #[prop(into)] title: String,
    #[prop(into)] detail: String,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <div class="flex flex-1 flex-col items-center justify-center gap-3 p-12 text-center">
            <div class="text-strong font-semibold">{title}</div>
            <p class="max-w-[46ch] text-body text-label-2">{detail}</p>
            {children.map(|c| c())}
        </div>
    }
}

/// A failed command. The cause chain is kept because for a broken manifest the
/// headline is generic and cargo's own diagnostic is two levels down.
#[component]
pub fn ErrorBanner(error: crate::ipc::IpcError, on_dismiss: Callback<()>) -> impl IntoView {
    view! {
        <div class="m-3 rounded-[10px] bg-crimson-fill px-3 py-2.5 ring-1 ring-crimson/25">
            <div class="flex items-start gap-2">
                <div class="mt-[5px]">
                    <Dot tone=Tone::Crimson />
                </div>
                <div class="min-w-0 flex-1 select-text">
                    <div class="text-body font-medium">{error.message}</div>
                    <ul class="mt-1 space-y-0.5 font-mono text-footnote leading-relaxed text-label-2">
                        {error
                            .causes
                            .into_iter()
                            .map(|cause| view! { <li class="whitespace-pre-wrap">"↳ "{cause}</li> })
                            .collect_view()}
                    </ul>
                </div>
                <button
                    type="button"
                    class="rounded-[4px] px-1.5 text-callout text-label-3 hover:text-label"
                    on:click=move |_| on_dismiss.run(())
                >
                    "✕"
                </button>
            </div>
        </div>
    }
}

/// A right-click menu: a panel at the pointer over a dismissing backdrop.
///
/// The chrome only. Every surface builds its own items, because a context
/// menu that is not about the thing under the pointer is just a worse main
/// menu — the whole reason to right-click is that the answer is local.
#[component]
pub fn ContextMenu(x: f64, y: f64, on_close: Callback<()>, children: Children) -> impl IntoView {
    view! {
        <div
            class="fixed inset-0 z-50"
            on:pointerdown=move |_| on_close.run(())
            on:contextmenu=move |event: leptos::ev::MouseEvent| {
                event.prevent_default();
                on_close.run(());
            }
        >
            <div
                class="absolute min-w-[200px] rounded-[8px] bg-raised py-1 shadow-2xl ring-1 ring-line-strong"
                style=format!("left: {x}px; top: {y}px")
                on:pointerdown=move |event: leptos::ev::PointerEvent| event.stop_propagation()
            >
                {children()}
            </div>
        </div>
    }
}

/// One row of a context menu.
#[component]
pub fn MenuItem(
    #[prop(into)] label: String,
    #[prop(optional, into)] shortcut: Option<String>,
    #[prop(optional)] danger: bool,
    #[prop(optional)] disabled: bool,
    on_select: Callback<()>,
) -> impl IntoView {
    let tone = if danger { "text-crimson" } else { "text-label-2" };
    view! {
        <button
            type="button"
            disabled=disabled
            on:click=move |_| on_select.run(())
            class=format!(
                "flex w-full items-center gap-8 px-3 py-1 text-left text-footnote {tone} \
                 hover:bg-selection hover:text-label disabled:pointer-events-none \
                 disabled:opacity-35",
            )
        >
            <span class="flex-1 truncate">{label}</span>
            {shortcut
                .map(|keys| {
                    view! {
                        <span class="shrink-0 font-mono text-caption text-label-4">{keys}</span>
                    }
                })}
        </button>
    }
}

/// A hairline between groups of menu items.
#[component]
pub fn MenuSeparator() -> impl IntoView {
    view! { <div class="my-1 h-px bg-line" /> }
}
