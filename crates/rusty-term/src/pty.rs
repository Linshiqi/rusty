//! A real terminal: a pseudo-terminal, an emulator, and a rendered screen.
//!
//! The earlier "terminal" here was a pipe. Pipes are enough for `cargo build`
//! and nothing else: no prompt appears, `Ctrl C` goes nowhere, and anything
//! that asks a question hangs forever with no way to answer it. A shell needs a
//! *pty* — a kernel object that looks like a keyboard and a screen to the
//! program on the other end.
//!
//! `portable-pty` supplies that, driving ConPTY on Windows, and `vt100` turns
//! the escape sequences that come back into a screen. Doing the emulation in
//! Rust rather than shipping xterm.js is what keeps this repository free of npm.

use std::{
    io::{Read, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::{
    error::{Error, Result},
    model::{Colour, Cursor, Row, Screen, Span, Style},
};

/// How many lines of history to keep.
///
/// A release build of a large workspace prints a few thousand lines, and the
/// whole point of scrollback is being able to find the first error after it has
/// scrolled away.
const SCROLLBACK: usize = 10_000;

/// The emulator and where the view is looking into it.
///
/// One mutex, not two. Held separately they were taken in opposite orders —
/// rendering locked the parser then the offset, scrolling locked the offset
/// then the parser — which is a deadlock waiting for someone to scroll back and
/// type at the same time.
struct Emulator {
    parser: vt100::Parser,
    /// How far back the view is scrolled, in lines above the live screen.
    scrollback: usize,
}

/// A running shell, its emulator, and the pipe back to it.
pub struct Terminal {
    emulator: Arc<Mutex<Emulator>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    /// Set once the child is gone, so the view can say so instead of quietly
    /// swallowing keystrokes.
    exited: Arc<Mutex<Option<i32>>>,
}

/// Signals that the screen changed.
///
/// Handed to exactly one consumer — the loop that renders and pushes frames —
/// rather than living on [`Terminal`], which is shared.
pub struct Updates {
    rx: Receiver<()>,
}

impl Updates {
    /// Block until something changed, then swallow whatever else arrived in the
    /// meantime.
    ///
    /// The coalescing is the point: a build scrolling at full speed produces
    /// thousands of writes a second, and rendering each one would spend the
    /// whole frame budget serialising screens nobody sees.
    pub fn wait(&self) -> bool {
        if self.rx.recv().is_err() {
            return false;
        }
        while self.rx.try_recv().is_ok() {}
        true
    }
}

impl Terminal {
    /// Start a shell in `cwd`.
    pub fn spawn(cwd: Option<&Path>, cols: u16, rows: u16) -> Result<(Terminal, Updates)> {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| Error::Pty(e.to_string()))?;

        let mut command = CommandBuilder::new(default_shell());
        if let Some(cwd) = cwd {
            command.cwd(cwd);
        }
        // Without this, programs that check `TERM` decide they are writing to a
        // file and turn off colour — which is most of the Rust toolchain.
        command.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| Error::Pty(e.to_string()))?;
        // Released as soon as the child owns it. On Unix this is what lets the
        // pty report end-of-file at all; on Windows the master keeps the
        // pseudoconsole alive regardless, which is why the exit is watched for
        // separately rather than inferred from the reader.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::Pty(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::Pty(e.to_string()))?;

        let emulator = Arc::new(Mutex::new(Emulator {
            parser: vt100::Parser::new(rows, cols, SCROLLBACK),
            scrollback: 0,
        }));
        let exited = Arc::new(Mutex::new(None));
        let child = Arc::new(Mutex::new(child));
        let (tx, rx) = mpsc::channel();

        let writer = Arc::new(Mutex::new(writer));
        pump(
            reader,
            Arc::clone(&emulator),
            Arc::clone(&writer),
            tx.clone(),
        );
        watch(Arc::clone(&child), Arc::clone(&exited), tx);

        Ok((
            Terminal {
                emulator,
                writer,
                master: Mutex::new(pair.master),
                child,
                exited,
            },
            Updates { rx },
        ))
    }

    /// Send keystrokes to the program.
    ///
    /// Any input scrolls back to the live screen, which is what every terminal
    /// does — typing into history would put the characters somewhere the cursor
    /// is not.
    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        self.scroll_to_bottom();
        let mut writer = self.writer.lock().expect("terminal writer");
        writer.write_all(bytes).map_err(Error::Write)?;
        writer.flush().map_err(Error::Write)
    }

    /// Tell both the pty and the emulator about a new size.
    ///
    /// Both, in that order. The program learns its width from the pty and the
    /// emulator has to agree, or a full-screen program draws to one width while
    /// the screen is stored at another.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        if cols == 0 || rows == 0 {
            return Ok(());
        }
        self.master
            .lock()
            .expect("terminal master")
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Pty(e.to_string()))?;
        self.emulator
            .lock()
            .expect("terminal emulator")
            .parser
            .screen_mut()
            .set_size(rows, cols);
        Ok(())
    }

    /// Move the view through history. Positive scrolls back.
    pub fn scroll(&self, delta: i32) {
        let mut emulator = self.emulator.lock().expect("terminal emulator");
        emulator.scrollback = emulator
            .scrollback
            .saturating_add_signed(delta as isize)
            .min(SCROLLBACK);
        let at = emulator.scrollback;
        emulator.parser.screen_mut().set_scrollback(at);
    }

    fn scroll_to_bottom(&self) {
        let mut emulator = self.emulator.lock().expect("terminal emulator");
        if emulator.scrollback != 0 {
            emulator.scrollback = 0;
            emulator.parser.screen_mut().set_scrollback(0);
        }
    }

    /// The current screen, ready to send.
    pub fn screen(&self) -> Screen {
        let emulator = self.emulator.lock().expect("terminal emulator");
        let exited = *self.exited.lock().expect("terminal exit");
        render(emulator.parser.screen(), emulator.scrollback, exited)
    }

    /// End the session.
    pub fn kill(&self) {
        let mut child = self.child.lock().expect("terminal child");
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Read the pty forever, feeding the emulator.
fn pump(
    mut reader: Box<dyn Read + Send>,
    emulator: Arc<Mutex<Emulator>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    tx: Sender<()>,
) {
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        // Holds the tail of a query split across two reads.
        let mut carry: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    // Answer before parsing. ConPTY opens by asking where the
                    // cursor is and will not start the shell until something
                    // replies, so a terminal that does not answer is a terminal
                    // that never prints anything at all.
                    carry.extend_from_slice(&buffer[..n]);
                    let cursor = {
                        let emulator = emulator.lock().expect("terminal emulator");
                        emulator.parser.screen().cursor_position()
                    };
                    let (replies, consumed) = answer_queries(&carry, cursor);
                    carry.drain(..consumed);
                    if !replies.is_empty() {
                        let mut writer = writer.lock().expect("terminal writer");
                        let _ = writer.write_all(&replies);
                        let _ = writer.flush();
                    }

                    emulator
                        .lock()
                        .expect("terminal emulator")
                        .parser
                        .process(&buffer[..n]);
                    if tx.send(()).is_err() {
                        // Nobody is rendering any more: the tab is gone.
                        return;
                    }
                }
            }
        }
    });
}

/// Watch for the shell exiting.
///
/// A separate thread, because end-of-file on the pty does not mean what it
/// means for a pipe: this side holds the master open, so on ConPTY the read
/// never returns zero however dead the child is. Waiting on the child itself is
/// the only signal that actually arrives.
///
/// Polled rather than blocking, so the mutex is never held across a wait — the
/// same lock is what `kill` needs to end the session.
fn watch(
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    exited: Arc<Mutex<Option<i32>>>,
    tx: Sender<()>,
) {
    thread::spawn(move || {
        loop {
            let status = child
                .lock()
                .expect("terminal child")
                .try_wait()
                .ok()
                .flatten();
            if let Some(status) = status {
                *exited.lock().expect("terminal exit") = Some(status.exit_code() as i32);
                // Wake the renderer once more so the view can say the shell is
                // gone rather than showing a live cursor over a dead screen.
                let _ = tx.send(());
                return;
            }
            if tx.send(()).is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(250));
        }
    });
}

/// Answer the queries a program will not start without.
///
/// ConPTY's first act is to ask the terminal where the cursor is — `ESC [ 6 n`
/// — and it blocks until something answers. `vt100` is a parser and has no
/// hook for it, so nothing in the pipeline replied and the shell simply never
/// ran: the pty produced exactly four bytes and then silence for ever. That is
/// the single reason the first version of this terminal was a blank rectangle.
///
/// Returns the bytes to send back and how much of `bytes` was consumed. The
/// remainder is a query split across two reads and must be kept.
fn answer_queries(bytes: &[u8], cursor: (u16, u16)) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    let mut at = 0;
    let mut consumed = 0;

    while at < bytes.len() {
        if bytes[at] != 0x1b {
            at += 1;
            consumed = at;
            continue;
        }
        // `ESC` with nothing after it yet: keep it for the next read.
        let Some(&b'[') = bytes.get(at + 1) else {
            if bytes.len() > at + 1 {
                at += 1;
                consumed = at;
                continue;
            }
            break;
        };

        let mut end = at + 2;
        while end < bytes.len() && matches!(bytes[end], b'0'..=b'9' | b';' | b'?') {
            end += 1;
        }
        // Ran out mid-sequence — wait for the rest.
        if end >= bytes.len() {
            break;
        }

        let params = &bytes[at + 2..end];
        match bytes[end] {
            // Cursor position. Terminals report it 1-based.
            b'n' if params == b"6" => {
                out.extend_from_slice(
                    format!("\x1b[{};{}R", cursor.0 + 1, cursor.1 + 1).as_bytes(),
                );
            }
            // "Are you there?" — the answer is "yes, and fine".
            b'n' if params == b"5" => out.extend_from_slice(b"\x1b[0n"),
            // Device attributes. Claiming to be a VT100 with an advanced video
            // option is what every emulator answers and what every program
            // expects; saying nothing leaves some of them waiting.
            b'c' if params.is_empty() || params == b"0" => {
                out.extend_from_slice(b"\x1b[?1;2c");
            }
            _ => {}
        }
        at = end + 1;
        consumed = at;
    }

    (out, consumed)
}

/// Turn the emulator's cell grid into runs of same-styled text.
fn render(screen: &vt100::Screen, scrollback: usize, exited: Option<i32>) -> Screen {
    let (rows, cols) = screen.size();
    let mut out = Vec::with_capacity(rows as usize);

    for row in 0..rows {
        let mut spans: Vec<Span> = Vec::new();
        for col in 0..cols {
            // The second half of a wide character is skipped outright: giving
            // it a space would add a column to every CJK line.
            let (text, style) = match screen.cell(row, col) {
                Some(cell) if cell.is_wide_continuation() => continue,
                Some(cell) if cell.has_contents() => (cell.contents(), style_of(cell)),
                Some(cell) => (" ", style_of(cell)),
                None => (" ", Style::default()),
            };

            match spans.last_mut() {
                Some(last) if last.style == style => last.text.push_str(text),
                _ => spans.push(Span {
                    text: text.to_string(),
                    style,
                }),
            }
        }

        // Trailing unstyled blanks carry no information and are most of a
        // typical screen. Dropping them roughly halves a frame.
        if let Some(last) = spans.last_mut()
            && last.style == Style::default()
        {
            let trimmed = last.text.trim_end_matches(' ');
            if trimmed.is_empty() {
                spans.pop();
            } else {
                last.text.truncate(trimmed.len());
            }
        }

        out.push(Row { spans });
    }

    // No cursor while scrolled back: drawn over history it would sit somewhere
    // the next keystroke does not go.
    let cursor = (!screen.hide_cursor() && scrollback == 0 && exited.is_none()).then(|| {
        let (row, col) = screen.cursor_position();
        Cursor { row, col }
    });

    Screen {
        rows: out,
        cols,
        cursor,
        scrollback,
        exited,
    }
}

fn style_of(cell: &vt100::Cell) -> Style {
    Style {
        fg: colour_of(cell.fgcolor()),
        bg: colour_of(cell.bgcolor()),
        bold: cell.bold(),
        italic: cell.italic(),
        underline: cell.underline(),
        inverse: cell.inverse(),
    }
}

fn colour_of(colour: vt100::Color) -> Colour {
    match colour {
        vt100::Color::Default => Colour::Default,
        vt100::Color::Idx(index) => Colour::Indexed { index },
        vt100::Color::Rgb(r, g, b) => Colour::Rgb { r, g, b },
    }
}

/// The shell to start.
///
/// PowerShell before `cmd` on Windows because that is what the Rust toolchain's
/// own instructions assume, and `$SHELL` on Unix because a user who changed it
/// meant it.
fn default_shell() -> String {
    if cfg!(windows) {
        for candidate in ["pwsh.exe", "powershell.exe"] {
            if which(candidate) {
                return candidate.to_string();
            }
        }
        return "cmd.exe".to_string();
    }

    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// Whether a program is on `PATH`.
///
/// Spawning to find out would flash a console window on Windows and cost a
/// process; walking `PATH` is what the shell itself does.
fn which(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A screen with nothing on it should not serialise 40 rows of spaces.
    #[test]
    fn blank_rows_carry_no_spans() {
        let parser = vt100::Parser::new(4, 20, 0);
        let screen = render(parser.screen(), 0, None);

        assert_eq!(screen.rows.len(), 4);
        assert!(
            screen.rows.iter().all(|row| row.spans.is_empty()),
            "trailing blanks must be trimmed away entirely",
        );
    }

    #[test]
    fn same_styled_neighbours_become_one_span() {
        let mut parser = vt100::Parser::new(2, 20, 0);
        parser.process(b"hello");
        let screen = render(parser.screen(), 0, None);

        assert_eq!(screen.rows[0].spans.len(), 1);
        assert_eq!(screen.rows[0].spans[0].text, "hello");
    }

    #[test]
    fn colour_changes_split_the_run() {
        let mut parser = vt100::Parser::new(2, 20, 0);
        // Red "err", then back to default "ok".
        parser.process(b"\x1b[31merr\x1b[0mok");
        let screen = render(parser.screen(), 0, None);

        let spans = &screen.rows[0].spans;
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert_eq!(spans[0].text, "err");
        assert_eq!(spans[0].style.fg, Colour::Indexed { index: 1 });
        assert_eq!(spans[1].text, "ok");
        assert_eq!(spans[1].style.fg, Colour::Default);
    }

    /// The query ConPTY blocks on, and the shape of the answer it wants.
    #[test]
    fn the_cursor_report_is_answered_one_based() {
        let (replies, consumed) = answer_queries(b"\x1b[6n", (0, 0));
        assert_eq!(replies, b"\x1b[1;1R");
        assert_eq!(consumed, 4);

        let (replies, _) = answer_queries(b"\x1b[6n", (11, 4));
        assert_eq!(replies, b"\x1b[12;5R");
    }

    #[test]
    fn ordinary_output_is_left_alone() {
        // Colour changes and text must pass through untouched, and be consumed
        // so the carry buffer does not grow without bound.
        let (replies, consumed) = answer_queries(b"\x1b[31mhello\x1b[0m", (0, 0));
        assert!(replies.is_empty());
        assert_eq!(consumed, 14);
    }

    /// A query split across two reads must not be answered twice, nor lost.
    #[test]
    fn a_split_query_waits_for_the_rest() {
        let (replies, consumed) = answer_queries(b"abc\x1b[6", (0, 0));
        assert!(replies.is_empty(), "half a query is not a query");
        assert_eq!(consumed, 3, "the incomplete tail must be kept");

        let (replies, consumed) = answer_queries(b"\x1b[6n", (0, 0));
        assert_eq!(replies, b"\x1b[1;1R");
        assert_eq!(consumed, 4);
    }

    #[test]
    fn device_attributes_get_an_identity() {
        let (replies, _) = answer_queries(b"\x1b[c", (0, 0));
        assert_eq!(replies, b"\x1b[?1;2c");
    }

    /// Scrolled back, the cursor must not be drawn: it would sit over history,
    /// nowhere near where the next keystroke lands.
    #[test]
    fn the_cursor_hides_while_scrolled_back() {
        let parser = vt100::Parser::new(4, 20, 100);
        assert!(render(parser.screen(), 0, None).cursor.is_some());
        assert!(render(parser.screen(), 5, None).cursor.is_none());
        assert!(render(parser.screen(), 0, Some(0)).cursor.is_none());
    }
}
