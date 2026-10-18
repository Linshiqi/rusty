//! rusty's desktop window, drawn with Slint. docs/slint.md is the design.
//!
//! `rusty [project] [file]` opens the project and the file in it when given;
//! `rusty --new-window` is New Window's, and opens on the welcome page.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod app;
mod calls;
mod catalogue;
mod diff_select;
mod docks;
mod editor;
mod filter;
mod font;
mod format;
mod instances;
mod keys;
mod language;
mod lsp;
mod markdown;
mod panes;
mod picker;
mod picture;
mod platform;
mod probe;
mod probe_history;
mod probe_output;
mod probe_surfaces;
mod probe_symbols;
mod probe_tabs;
mod probe_testfail;
mod project;
mod reveal;
mod run;
mod symbols;
mod tab_drag;
mod targets;
mod trail;
mod tree;
mod work;
mod zoom;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    // Opened from Finder or the Dock, a macOS app runs on launchd's PATH and
    // not the shell's, and cargo lives on the shell's. First, before a thread
    // exists: every spawn in the process reads the environment it sets.
    rusty_embed::tools::adopt_shell_bins();
    // Before any window: a Mac's keeps the system's frame, drawn under
    // rusty's own title bar, and no menu bar whose Quit bypasses `closing()`.
    platform::select_backend()?;
    probe::mark("main");
    if let Err(refusal) = probe::ready() {
        eprintln!("rusty: {refusal}");
        std::process::exit(2);
    }
    // Started by another window's New Window: the welcome page, not the
    // project the last session had, which is open in the window that asked.
    let fresh = std::env::args()
        .skip(1)
        .any(|arg| arg == instances::NEW_WINDOW);
    let mut args = std::env::args()
        .skip(1)
        .filter(|arg| arg != instances::NEW_WINDOW);
    // Absolute, as every path the window keeps is: a relative root gave
    // the language server `file://examples/…`, which it refuses as no file.
    // Not canonicalised — on Windows that is a `\\?\` path, which tools
    // handed it cannot append to.
    let root = args
        .next()
        .map(|arg| std::path::absolute(&arg).unwrap_or_else(|_| arg.into()));
    let file = args.next();

    // The tools the installer ships — the emulator, the debuggers, the
    // flasher — under `bundled/` beside the executable, in the app
    // bundle's Resources on macOS, or under `lib/rusty` where a package
    // puts the program in `bin`; `target/bundled` in a checkout, where
    // `scripts/bundle-tools.sh` writes them. They join the finder's ladder
    // after anything the user installed.
    if let Some(dir) = bundled_dir() {
        rusty_embed::tools::set_bundled_dir(dir);
    }
    // Counted by the other rusty windows on this machine.
    instances::register();
    probe::mark("registered");
    let saved = rusty_embed::config::workbench();
    probe::mark("workbench read");
    let window = AppWindow::new()?;
    probe::mark("window made");
    // After the window: Slint's translations are bundled with the component,
    // and asked for before it exists the choice is lost and Slint follows the
    // system — the two halves of the window then spoke two languages.
    app::apply_locale(&app::chosen_locale(saved.locale.as_deref()));
    probe::mark("locale");
    // Before the editor measures a character: the font it measures with
    // has to be one this machine draws monospaced.
    font::choose(&window);
    platform::mark(&window);
    probe::mark("font");
    let app = app::App::new(&window);
    probe::mark("app made");
    work::install(app.clone());
    {
        let app = app.clone();
        window.window().on_close_requested(move || app.closing());
    }
    window.show()?;
    probe::mark("shown");
    zoom::apply(&window, saved.layout.zoom);
    window.invoke_focus_window();

    match root {
        Some(root) => {
            // The file in it, as the project names it: a path given whole is
            // taken relative to the project when it is inside it, and left
            // for the project to refuse when it is not.
            let file = file.map(|file| {
                std::path::Path::new(&file)
                    .strip_prefix(&root)
                    .map(|inside| inside.to_string_lossy().into_owned())
                    .unwrap_or(file)
                    .replace('\\', "/")
            });
            app.open_project(root, file)
        }
        // A new window: the welcome page, and the machine was asked about
        // by the window that opened it.
        None if fresh => {}
        // No project named: the one worked on last, as it was left, when it
        // is still there and no other window has it open. Without one the
        // machine is still worth asking about: a first run with no Rust
        // says so before anybody opens anything.
        None => match saved
            .recent_projects
            .first()
            .map(std::path::PathBuf::from)
            .filter(|root| root.is_dir() && !instances::holding(root))
        {
            Some(root) => app.open_project(root, None),
            None => app.act("environment.refresh"),
        },
    }
    probe::mark("project asked");
    probe::start(&window);
    slint::Timer::single_shot(std::time::Duration::ZERO, || {
        probe::mark("event loop turned")
    });
    slint::run_event_loop()
}

/// Where the installed tools are, of the places an install puts them.
fn bundled_dir() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [
        dir.join("bundled"),
        dir.join("../Resources/bundled"),
        dir.join("../lib/rusty/bundled"),
        dir.join("../bundled"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_dir())
}
