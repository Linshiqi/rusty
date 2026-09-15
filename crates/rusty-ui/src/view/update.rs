//! The update sheet: what a newer rusty changes, and one button to get it.
//!
//! Opened by the launch check when the feed names a version newer than this
//! build, and by Help ▸ Check for updates whatever the answer — a menu item
//! that sometimes does nothing is one people stop trusting. The notes are the
//! CHANGELOG section the release was published with, drawn by the same
//! Markdown page that draws a chapter, because they were written for the
//! person reading this sheet and for nobody else.
//!
//! Download and restart are two gestures on purpose. The installer is a
//! hundred megabytes; it downloads while the user keeps working, its
//! signature is checked before a byte of it is kept, and the restart waits
//! for them — an update that restarted the workbench the moment its download
//! happened to finish would take an unsaved edit with it.

use leptos::{ev, prelude::*};

use rusty_i18n::t;

use crate::{
    controller,
    state::{AppState, UpdateStage},
    view::{
        components::{Button, ButtonKind, Pill, Tone},
        markdown::Markdown,
    },
};

/// Bytes as the installer's own unit, one decimal, so a progress line reads
/// `41.3 MB of 95.0 MB` rather than a nine-digit count.
pub(crate) fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

#[component]
pub fn UpdateSheet() -> impl IntoView {
    let state = AppState::expect();

    move || {
        if !state.app.update_open.get() {
            return None;
        }
        let status = state.app.update.get()?;
        let stage = state.app.update_stage.get();
        let busy = matches!(stage, UpdateStage::Downloading | UpdateStage::Applying);

        let latest = status.latest.clone().unwrap_or_default();
        let title = if status.newer {
            t!("update.title-available", version = latest.clone())
        } else if status.note.is_some() {
            t!("update.title-unreachable")
        } else {
            t!("update.title-newest")
        };

        let body = if status.newer {
            let notes = match status.notes.clone() {
                Some(notes) => view! { <Markdown text=notes /> }.into_any(),
                None => view! {
                    <p class="text-footnote text-label-3">{t!("update.notes-none")}</p>
                }
                .into_any(),
            };
            view! {
                <p class="text-footnote text-label-2">
                    {t!("update.summary", current = status.current.clone())}
                    {status
                        .date
                        .clone()
                        .map(|date| view! { <span class="text-label-3">{format!(" · {}", t!("update.published", date = date))}</span> })}
                </p>
                <div class="max-h-[46vh] overflow-y-auto rounded-[8px] bg-sunken px-4 py-3">
                    {notes}
                </div>
            }
            .into_any()
        } else if let Some(note) = status.note.clone() {
            view! { <p class="text-footnote text-label-2 select-text">{note}</p> }.into_any()
        } else {
            view! {
                <p class="text-footnote text-label-2">
                    {t!("update.newest-detail", current = status.current.clone())}
                </p>
            }
            .into_any()
        };

        // Where the download is, when there is one: a bar with an end when
        // the server said how long, activity when it did not, and a plain
        // sentence once the bytes are verified.
        let progress = match stage {
            UpdateStage::Downloading => {
                let (fraction, label) = match state.app.update_progress.get() {
                    Some(p) => match p.total {
                        Some(total) if total > 0 => (
                            Some((p.received as f64 / total as f64).min(1.0)),
                            t!(
                                "update.downloading",
                                done = megabytes(p.received),
                                total = megabytes(total)
                            ),
                        ),
                        _ => (
                            None,
                            t!("update.downloading-unsized", done = megabytes(p.received)),
                        ),
                    },
                    None => (None, t!("update.starting")),
                };
                let (width, pulse) = match fraction {
                    Some(fraction) => (format!("width:{:.1}%", fraction * 100.0), ""),
                    None => ("width:100%".to_string(), " animate-pulse"),
                };
                Some(
                    view! {
                        <div class="flex flex-col gap-1.5">
                            <div class="h-1.5 w-full overflow-hidden rounded-full bg-sunken">
                                <div
                                    class=format!("h-full rounded-full bg-rust transition-[width] duration-200{pulse}")
                                    style=width
                                ></div>
                            </div>
                            <div class="text-caption text-label-3">{label}</div>
                        </div>
                    }
                    .into_any(),
                )
            }
            UpdateStage::Ready => Some(
                view! {
                    <div class="flex items-center gap-2">
                        <Pill label=t!("update.ready") tone=Tone::Patina />
                    </div>
                }
                .into_any(),
            ),
            UpdateStage::Applying => Some(
                view! { <div class="text-caption text-label-3">{t!("update.applying")}</div> }
                    .into_any(),
            ),
            _ => None,
        };

        let buttons = if !status.newer {
            view! {
                {status
                    .note
                    .is_some()
                    .then(|| {
                        view! {
                            <Button
                                label=t!("update.releases")
                                on_click=Callback::new(move |_| {
                                    controller::open_url(state, rusty_embed::REPO_RELEASES.to_string())
                                })
                            />
                        }
                    })}
                <Button
                    label=t!("update.close")
                    kind=ButtonKind::Primary
                    on_click=Callback::new(move |_| controller::dismiss_update(state))
                />
            }
            .into_any()
        } else {
            match stage {
                UpdateStage::Downloading => view! {
                    <Button
                        label=t!("update.cancel")
                        on_click=Callback::new(move |_| controller::cancel_update(state))
                    />
                }
                .into_any(),
                UpdateStage::Ready => view! {
                    <Button
                        label=t!("update.later")
                        on_click=Callback::new(move |_| controller::dismiss_update(state))
                    />
                    <Button
                        label=t!("update.restart")
                        kind=ButtonKind::Primary
                        on_click=Callback::new(move |_| controller::apply_update(state))
                    />
                }
                .into_any(),
                UpdateStage::Applying => ().into_any(),
                UpdateStage::Idle | UpdateStage::Checking => view! {
                    <Button
                        label=t!("update.skip")
                        kind=ButtonKind::Quiet
                        on_click=Callback::new(move |_| controller::skip_update(state))
                    />
                    <div class="flex-1"></div>
                    <Button
                        label=t!("update.later")
                        on_click=Callback::new(move |_| controller::dismiss_update(state))
                    />
                    <Button
                        label=t!("update.download")
                        kind=ButtonKind::Primary
                        on_click=Callback::new(move |_| controller::download_update(state))
                    />
                }
                .into_any(),
            }
        };

        Some(view! {
            <div
                class="absolute inset-0 z-40 flex items-center justify-center bg-canvas/80 p-8"
                on:keydown=move |event: ev::KeyboardEvent| {
                    if event.key() == "Escape" && !busy {
                        controller::dismiss_update(state);
                    }
                }
            >
                // The stage, readable off the DOM: what a driven test
                // asserts on, since the signal itself is out of its reach.
                <div
                    role="dialog"
                    data-stage=format!("{stage:?}")
                    class="flex w-[640px] max-w-full flex-col gap-4 rounded-[10px] border border-line bg-content p-5 shadow-2xl"
                >
                    <div class="text-title font-semibold">{title}</div>
                    {body}
                    {progress}
                    <div class="flex items-center justify-end gap-2">{buttons}</div>
                </div>
            </div>
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_the_installers_own_unit() {
        assert_eq!(megabytes(0), "0.0 MB");
        assert_eq!(megabytes(1024 * 1024), "1.0 MB");
        // The Windows installer as published: a nine-digit count nobody reads.
        assert_eq!(megabytes(99_586_610), "95.0 MB");
    }
}
