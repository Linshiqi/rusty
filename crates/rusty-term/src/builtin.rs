//! The built-in shell.
//!
//! Compiled into rusty itself and started as `rusty --builtin-shell` inside
//! the pty, so the terminal works the moment the app is installed — no
//! download, no PowerShell profile spending two seconds before the prompt,
//! and the same language on every OS.
//!
//! The language is bash. Execution is `brush-core`, a POSIX/bash-compatible
//! shell written in Rust — the released 0.5, which carries the Windows PATH
//! fix this crate once vendored a patched copy for. Pipes, redirection,
//! globs, variables,
//! command substitution, conditionals and loops all behave; external
//! commands — the cargo, espflash, git and gdb this workbench is about —
//! spawn as real processes inheriting the pty.
//!
//! Only the line editing is ours: echo, backspace, Ctrl+C, and arrow-key
//! history over the raw VT bytes ConPTY delivers. brush's own interactive
//! frontend assumes it owns a real console; inside a pty slave the simple
//! loop is the reliable one.

use std::io::{Read, Write};

use unicode_width::UnicodeWidthChar;

/// Run the shell until EOF or `exit`. Never returns to the caller.
pub fn run() -> ! {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a single-threaded runtime");
    let code = runtime.block_on(repl());
    std::process::exit(code);
}

/// The console, taken raw — the first thing every real shell does.
///
/// Inside ConPTY the child's console still defaults to cooked mode: conhost
/// line-buffers and echoes on its own, so every command appeared twice and
/// arrow keys never reached us as VT bytes. Interactive mode turns all of
/// that off and asks for VT input; executing mode re-enables Ctrl+C
/// *processing* so a running cargo can be interrupted, while a handler keeps
/// the same event from killing the shell around it.
#[cfg(windows)]
// FFI is the only way to set a console mode; the crate that wraps it safely
// (crossterm) is a whole TUI toolkit for what is two SetConsoleMode calls.
#[allow(unsafe_code)]
mod console {
    use windows_sys::Win32::System::Console::{
        CTRL_C_EVENT, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
        ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode,
        GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleCtrlHandler, SetConsoleMode,
    };

    unsafe extern "system" fn swallow_ctrl_c(kind: u32) -> i32 {
        // The child in the same console gets the event too and dies of it;
        // the shell must not. Everything else (close, logoff) proceeds.
        i32::from(kind == CTRL_C_EVENT)
    }

    pub struct Console {
        stdin: isize,
        raw: u32,
    }

    pub fn take() -> Option<Console> {
        unsafe {
            let stdin = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode = 0u32;
            if GetConsoleMode(stdin, &mut mode) == 0 {
                // Not a console (a plain pipe, as in tests): nothing to set,
                // and nothing needs setting.
                return None;
            }
            let raw = (mode & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT))
                | ENABLE_VIRTUAL_TERMINAL_INPUT;
            SetConsoleMode(stdin, raw);

            let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut out_mode = 0u32;
            if GetConsoleMode(stdout, &mut out_mode) != 0 {
                SetConsoleMode(stdout, out_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }

            SetConsoleCtrlHandler(Some(swallow_ctrl_c), 1);
            Some(Console {
                stdin: stdin as isize,
                raw,
            })
        }
    }

    impl Console {
        pub fn interactive(&self) {
            unsafe {
                SetConsoleMode(self.stdin as *mut _, self.raw);
            }
        }
        pub fn executing(&self) {
            unsafe {
                SetConsoleMode(self.stdin as *mut _, self.raw | ENABLE_PROCESSED_INPUT);
            }
        }
    }
}

/// The Unix twin: raw termios while reading, signals back on while a child
/// runs. The shell ignores SIGINT while it is reading a line, as bash does;
/// while a child runs the disposition is put back to the default, because a
/// `SIG_IGN` is *inherited across exec* — POSIX says so, and brush never
/// resets it — so a child spawned under the ignore would have ignored Ctrl+C
/// too. The earlier comment here claimed the child reset it; it does not.
#[cfg(unix)]
// Same grounds as the Windows twin: termios is FFI or nothing.
#[allow(unsafe_code)]
mod console {
    pub struct Console {
        base: libc::termios,
    }

    pub fn take() -> Option<Console> {
        unsafe {
            let mut base: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut base) != 0 {
                return None;
            }
            let console = Console { base };
            console.interactive();
            Some(console)
        }
    }

    impl Console {
        pub fn interactive(&self) {
            unsafe {
                let mut raw = self.base;
                raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
                libc::tcsetattr(0, libc::TCSANOW, &raw);
                libc::signal(libc::SIGINT, libc::SIG_IGN);
            }
        }
        pub fn executing(&self) {
            unsafe {
                let mut cooked = self.base;
                cooked.c_lflag &= !(libc::ICANON | libc::ECHO);
                cooked.c_lflag |= libc::ISIG;
                libc::tcsetattr(0, libc::TCSANOW, &cooked);
                // Default, so the children spawned from here on inherit a
                // Ctrl+C that interrupts them. The shell itself is back in
                // `interactive` — and back to ignoring — before it reads
                // the next line.
                libc::signal(libc::SIGINT, libc::SIG_DFL);
            }
        }
    }
}

async fn repl() -> i32 {
    let mut stdout = std::io::stdout();
    let terminal = console::take();
    if let Some(console) = &terminal {
        console.interactive();
    }

    let builtins = brush_builtins::default_builtins::<brush_core::extensions::DefaultShellExtensions>(
        brush_builtins::BuiltinSet::BashMode,
    );
    let mut shell = match brush_core::Shell::builder()
        .builtins(builtins)
        // The one coreutil navigation cannot live without, and the bash set
        // does not carry: our own, identical on every OS.
        .builtin(
            "ls",
            brush_core::builtins::builtin::<
                LsCommand,
                brush_core::extensions::DefaultShellExtensions,
            >(),
        )
        .build()
        .await
    {
        Ok(shell) => shell,
        Err(error) => {
            let _ = writeln!(stdout, "the shell engine failed to start: {error}\r");
            return 1;
        }
    };

    let stdin = std::io::stdin();
    let mut bytes = stdin.lock().bytes();
    let mut history: Vec<String> = Vec::new();

    let _ = writeln!(stdout, "rusty shell — bash syntax, built in.\r");

    loop {
        prompt(&mut stdout, shell.working_dir());
        let Some(line) = read_line(&mut bytes, &mut stdout, &history) else {
            let _ = writeln!(stdout, "\r");
            return 0;
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if history.last() != Some(&line) {
            history.push(line.clone());
        }

        // `clear` before brush sees it: there is no terminfo in here, and
        // the two escape bytes are all a clear ever was.
        if line == "clear" {
            let _ = write!(stdout, "\x1b[2J\x1b[H");
            let _ = stdout.flush();
            continue;
        }

        let params = shell.default_exec_params();
        // Cooked enough for Ctrl+C to interrupt what runs; raw again for us.
        if let Some(console) = &terminal {
            console.executing();
        }
        let outcome = shell
            .run_string(line, &brush_core::SourceInfo::default(), &params)
            .await;
        if let Some(console) = &terminal {
            console.interactive();
        }
        match outcome {
            Ok(result) => {
                if matches!(
                    result.next_control_flow,
                    brush_core::ExecutionControlFlow::ExitShell,
                ) {
                    return if result.is_success() { 0 } else { 1 };
                }
                if !result.is_success() {
                    // The prompt carries no exit-code segment; one quiet
                    // line keeps failures from passing silently.
                    let code = u8::from(&result.exit_code);
                    let _ = writeln!(stdout, "\x1b[31m— exit {code}\x1b[0m\r");
                }
            }
            Err(error) => {
                let _ = writeln!(stdout, "\x1b[31m{error}\x1b[0m\r");
            }
        }
    }
}

fn prompt(stdout: &mut impl Write, cwd: &std::path::Path) {
    // The last component is enough orientation; `pwd` has the rest. Cyan,
    // then reset — the vt100 layer renders it like any other program's.
    let name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.display().to_string());
    let _ = write!(stdout, "\x1b[36m{name}\x1b[0m$ ");
    let _ = stdout.flush();
}

/// One line, edited live: echo, backspace, arrow-key history. `None` on EOF
/// or Ctrl+D at an empty line.
fn read_line(
    bytes: &mut impl Iterator<Item = std::io::Result<u8>>,
    stdout: &mut impl Write,
    history: &[String],
) -> Option<String> {
    let mut line = String::new();
    // Where the arrows are in history; one past the end means "the line
    // being typed".
    let mut at = history.len();

    loop {
        let byte = bytes.next()?.ok()?;
        match byte {
            b'\r' | b'\n' => {
                let _ = write!(stdout, "\r\n");
                let _ = stdout.flush();
                return Some(line);
            }
            // Ctrl+C: abandon the line, keep the shell.
            0x03 => {
                let _ = write!(stdout, "^C\r\n");
                let _ = stdout.flush();
                line.clear();
                return Some(line);
            }
            // Ctrl+D on an empty line is EOF, as everywhere.
            0x04 if line.is_empty() => return None,
            0x7f | 0x08 => {
                if let Some(gone) = line.pop() {
                    rub_out(stdout, cells(gone));
                    let _ = stdout.flush();
                }
            }
            // ESC: an arrow or another CSI. The whole sequence is consumed
            // (`read_csi`) and only the arrows are acted on; everything else
            // is dropped rather than echoed as garbage.
            0x1b => {
                let Some((params, code)) = read_csi(bytes) else {
                    continue;
                };
                // A modified arrow (`1;5A`) is not a history walk.
                if !params.is_empty() {
                    continue;
                }
                let replacement = match code {
                    b'A' if at > 0 => {
                        at -= 1;
                        Some(history[at].clone())
                    }
                    b'B' if at < history.len() => {
                        at += 1;
                        Some(if at == history.len() {
                            String::new()
                        } else {
                            history[at].clone()
                        })
                    }
                    _ => None,
                };
                if let Some(next) = replacement {
                    repaint_line(stdout, &line, &next);
                    line = next;
                }
            }
            // UTF-8 lead bytes accumulate to a whole scalar before echoing;
            // echo per byte would tear glyphs.
            _ if !byte.is_ascii() => push_utf8(&mut line, byte, bytes, stdout),
            // Unhandled control bytes are dropped, not stored: a Tab pushed
            // invisibly into the line made commands that looked right carry
            // a byte nobody could see.
            _ if byte.is_ascii_control() => {}
            _ => {
                line.push(byte as char);
                let _ = stdout.write_all(&[byte]);
                let _ = stdout.flush();
            }
        }
    }
}

/// The rest of a CSI sequence whose `ESC` has just been read: its parameter
/// bytes and its final byte. The whole sequence is consumed — parameter bytes
/// (`0x30..=0x3F`, so `3` and `1;5`), then intermediates, then the one final
/// byte. Taking exactly one byte after `[` left Delete's `~` and Ctrl+Arrow's
/// `;5C` sitting in the command line.
///
/// `None` when the byte after `ESC` is not `[`, or when the input ends or
/// fails before the final byte; what was read is dropped either way.
fn read_csi(bytes: &mut impl Iterator<Item = std::io::Result<u8>>) -> Option<(Vec<u8>, u8)> {
    let Some(Ok(b'[')) = bytes.next() else {
        return None;
    };
    let mut params: Vec<u8> = Vec::new();
    loop {
        match bytes.next() {
            Some(Ok(byte @ 0x30..=0x3F)) => params.push(byte),
            Some(Ok(0x20..=0x2F)) => {}
            Some(Ok(byte)) => return Some((params, byte)),
            _ => return None,
        }
    }
}

/// Repaint: wipe the line as it stands, write `next` in its place.
fn repaint_line(stdout: &mut impl Write, line: &str, next: &str) {
    rub_out(stdout, line.chars().map(cells).sum());
    let _ = write!(stdout, "{next}");
    let _ = stdout.flush();
}

/// How many cells `c` takes on screen, measured the way vt100 — the
/// terminal the pty is drawn by — measures it: a character at a time, by
/// the same table. `中` is two, `ž` one, a combining accent none of its own
/// (it is drawn into the cell before it) and a control character none.
///
/// Backspace used to call anything past U+00FF two cells wide, so `ž`
/// rubbed out the cell before it too, and recalling history rubbed out one
/// cell a character, which left half of a CJK line on the screen.
fn cells(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Rub out the `cells` cells before the cursor, leaving it on the first:
/// back, a space over the cell, back again, for each.
fn rub_out(stdout: &mut impl Write, cells: usize) {
    for _ in 0..cells {
        let _ = write!(stdout, "\x08 \x08");
    }
}

/// Accumulate the rest of a UTF-8 scalar whose lead byte just arrived, then
/// echo the whole glyph at once.
fn push_utf8(
    line: &mut String,
    lead: u8,
    bytes: &mut impl Iterator<Item = std::io::Result<u8>>,
    stdout: &mut impl Write,
) {
    let need = match lead {
        0xC0..=0xDF => 1,
        0xE0..=0xEF => 2,
        0xF0..=0xF7 => 3,
        _ => return,
    };
    let mut buffer = vec![lead];
    for _ in 0..need {
        match bytes.next() {
            Some(Ok(byte)) => buffer.push(byte),
            _ => return,
        }
    }
    if let Ok(text) = std::str::from_utf8(&buffer) {
        line.push_str(text);
        let _ = stdout.write_all(&buffer);
        let _ = stdout.flush();
    }
}

/// `ls`, the same on every OS: directories first and marked, then files.
/// Consistency across platforms is the point of carrying our own.
#[derive(clap::Parser)]
struct LsCommand {
    /// Directories to list; the working directory when empty.
    paths: Vec<String>,
}

impl brush_core::builtins::Command for LsCommand {
    type Error = brush_core::Error;

    async fn execute<SE: brush_core::ShellExtensions>(
        &self,
        context: brush_core::ExecutionContext<'_, SE>,
    ) -> Result<brush_core::ExecutionResult, Self::Error> {
        let base = context.shell.working_dir().to_path_buf();
        let targets: Vec<String> = if self.paths.is_empty() {
            vec![".".to_string()]
        } else {
            self.paths.clone()
        };
        let mut failed = false;
        let many = targets.len() > 1;
        for (index, target) in targets.iter().enumerate() {
            let dir = {
                let candidate = std::path::PathBuf::from(target);
                if candidate.is_absolute() {
                    candidate
                } else {
                    base.join(candidate)
                }
            };
            if many {
                if index > 0 {
                    let _ = writeln!(context.stdout());
                }
                let _ = writeln!(context.stdout(), "{target}:");
            }
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(error) => {
                    let _ = writeln!(context.stderr(), "ls: {target}: {error}");
                    failed = true;
                    continue;
                }
            };
            let mut names: Vec<(bool, String)> = entries
                .flatten()
                .map(|entry| {
                    let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
                    (is_dir, entry.file_name().to_string_lossy().into_owned())
                })
                .collect();
            names.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
            });
            for (is_dir, name) in names {
                if is_dir {
                    let _ = writeln!(context.stdout(), "\x1b[36m{name}/\x1b[0m");
                } else {
                    let _ = writeln!(context.stdout(), "{name}");
                }
            }
        }
        Ok(if failed {
            brush_core::ExecutionResult::new(1)
        } else {
            brush_core::ExecutionResult::success()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One cell rubbed out: back, a space over it, back again.
    const ERASE: &str = "\x08 \x08";

    /// What editing a line writes to the terminal for `input`, as text, and
    /// the line it hands back.
    fn typed(input: &str, history: &[&str]) -> (String, Option<String>) {
        let history: Vec<String> = history.iter().map(|line| line.to_string()).collect();
        let mut bytes = input.bytes().map(std::io::Result::Ok);
        let mut written = Vec::new();
        let line = read_line(&mut bytes, &mut written, &history);
        (String::from_utf8(written).expect("UTF-8 out"), line)
    }

    /// Backspace rubs out the cells the character took: two for `中`, and one
    /// for `ž`, which the old rule — anything past U+00FF is wide — gave two,
    /// rubbing out the cell before it as well.
    #[test]
    fn backspace_rubs_out_the_cells_the_character_took() {
        let (written, line) = typed("中\x7f\r", &[]);
        assert_eq!(written, format!("中{}\r\n", ERASE.repeat(2)));
        assert_eq!(line.as_deref(), Some(""));

        let (written, line) = typed("ž\x7f\r", &[]);
        assert_eq!(written, format!("ž{ERASE}\r\n"));
        assert_eq!(line.as_deref(), Some(""));
    }

    /// Up rubs out the line it replaces cell by cell, and a CJK character is
    /// two of them: counted by characters, `中文` left half of itself on the
    /// screen beside the line recalled over it — typed or recalled alike.
    #[test]
    fn recalling_a_line_rubs_out_every_cell_of_the_one_it_replaces() {
        let (written, line) = typed("中文\x1b[A\x1b[A\r", &["ls", "echo 中文"]);
        assert_eq!(
            written,
            format!("中文{}echo 中文{}ls\r\n", ERASE.repeat(4), ERASE.repeat(9))
        );
        assert_eq!(line.as_deref(), Some("ls"));
    }
}
