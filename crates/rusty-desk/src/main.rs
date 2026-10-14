//! rusty's desktop window, drawn with Slint. docs/slint.md is the design.
//!
//! `rusty [project] [file]` opens the project and the file in it when given.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod app;
mod catalogue;
mod docks;
mod editor;
mod format;
mod keys;
mod language;
mod lsp;
mod picker;
mod probe;
mod project;
mod run;
mod work;
mod zoom;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    if let Err(refusal) = probe::ready() {
        eprintln!("rusty: {refusal}");
        std::process::exit(2);
    }
    let mut args = std::env::args().skip(1);
    let root = args.next().map(std::path::PathBuf::from);
    let file = args.next();

    let saved = rusty_embed::config::workbench();
    let window = AppWindow::new()?;
    // After the window: Slint's translations are bundled with the component,
    // and asked for before it exists the choice is lost and Slint follows the
    // system — the two halves of the window then spoke two languages.
    app::apply_locale(&app::chosen_locale(saved.locale.as_deref()));
    let app = app::App::new(&window);
    work::install(app.clone());
    {
        let app = app.clone();
        window.window().on_close_requested(move || app.closing());
    }
    window.show()?;
    zoom::apply(&window, saved.layout.zoom);
    window.invoke_focus_window();

    match root {
        Some(root) => app.open_project(root, file.map(|file| file.replace('\\', "/"))),
        // Without a project the machine is still worth asking about: a
        // first run with no Rust says so before anybody opens anything.
        None => app.act("environment.refresh"),
    }
    probe::start(&window);
    slint::run_event_loop()
}
