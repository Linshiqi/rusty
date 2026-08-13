//! Drive the built-in shell through a real ConPTY, the way the app does —
//! the pipe smoke test cannot see console-mode problems, and this can.
//! Usage: cargo run -p rusty-term --example pty_probe

fn main() {
    // Default: the built-in shell. Any argv can be passed instead —
    // `--example pty_probe -- powershell.exe` probes that shell.
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        let exe = std::env::current_dir()
            .unwrap()
            .join("target/debug/rusty-app.exe");
        argv = vec![
            exe.to_string_lossy().into_owned(),
            "--builtin-shell".to_string(),
        ];
    }
    // RUSTY_PROBE_CWD reproduces the app's working-directory choice.
    let cwd = std::env::var("RUSTY_PROBE_CWD").ok().map(std::path::PathBuf::from);
    let (terminal, updates) =
        rusty_term::Terminal::spawn(cwd.as_deref(), 100, 24, Some(&argv)).expect("spawn");

    let start = std::time::Instant::now();
    std::thread::spawn(move || while updates.wait() {});

    std::thread::sleep(std::time::Duration::from_millis(1200));
    terminal.write(b"ls\r").expect("write");
    std::thread::sleep(std::time::Duration::from_millis(1500));

    let screen = terminal.screen();
    println!("--- exited: {:?}  (after {:?})", screen.exited, start.elapsed());
    for row in &screen.rows {
        let text: String = row.spans.iter().map(|s| s.text.as_str()).collect();
        if !text.trim().is_empty() {
            println!("| {text}");
        }
    }
    terminal.kill();
}
