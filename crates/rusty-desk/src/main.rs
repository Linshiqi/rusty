//! rusty's desktop window, drawn with Slint.

mod editor;
mod probe;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let file = args
        .next()
        .unwrap_or_else(|| "crates/rusty-ui/src/view/panels/files/surface/mod.rs".into());
    let window = AppWindow::new()?;
    let _surface = editor::Surface::open(&window, &root, &file);
    probe::start(&window);
    window.run()
}
