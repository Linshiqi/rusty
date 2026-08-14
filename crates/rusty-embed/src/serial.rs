//! A serial port rusty holds open itself, in both directions.
//!
//! Everywhere else this workbench drives somebody else's tool rather than
//! reimplementing it, and monitoring is normally `espflash monitor` — which
//! decodes defmt and resolves panic addresses, neither of which rusty should
//! reinvent. But espflash reads the keyboard through crossterm's console
//! events, not through its stdin, so a monitor rusty spawned is one-way: what
//! the board says arrives, and nothing can be said back to it.
//!
//! A tuning loop needs the other direction. A port can only be open once, so
//! the two cannot both hold it — this is the mode for turning a gain while the
//! craft is in the air, and `espflash monitor` stays the mode for reading a
//! defmt log or a backtrace. The panel says which trade it is making.

use std::{
    io::{ErrorKind, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

use crate::{
    error::{Error, Result},
    model::{LogLine, LogStream},
    process::{Input, Stopper},
};

/// How long a read waits before looking at the stop flag again. Short enough
/// that closing feels instant, long enough not to spin a core on an idle port.
const POLL: Duration = Duration::from_millis(100);

/// An open port, its reader thread, and the way to write back to it.
pub struct Link {
    lines: Receiver<LogLine>,
    input: Input,
    open: Arc<AtomicBool>,
}

impl Link {
    /// The next line, or `None` once the link is closed and drained.
    pub fn recv(&self) -> Option<LogLine> {
        self.lines.recv().ok()
    }

    pub fn input(&self) -> Input {
        self.input.clone()
    }

    /// Ends the link from outside the reader loop, exactly as a spawned tool's
    /// stopper does — so a serial link and a monitor process are the same kind
    /// of session to everything above this.
    pub fn stopper(&self) -> Stopper {
        let open = Arc::clone(&self.open);
        Stopper::new(move || open.store(false, Ordering::Relaxed))
    }
}

/// Open a port for reading and writing.
///
/// A port already held by something else fails here rather than half-working
/// later: on Windows that is an access-denied that reads like a driver fault,
/// so the error names the likely cause instead.
pub fn open(port: &str, baud: u32) -> Result<Link> {
    let handle = serialport::new(port, baud)
        .timeout(POLL)
        .open()
        .map_err(|source| Error::SerialPort {
            port: port.to_string(),
            message: source.description,
        })?;
    let writer = handle.try_clone().map_err(|source| Error::SerialPort {
        port: port.to_string(),
        message: source.description,
    })?;

    let open = Arc::new(AtomicBool::new(true));
    let (tx, lines) = mpsc::channel();
    crate::process::pump(
        Wire {
            reader: handle,
            open: Arc::clone(&open),
        },
        LogStream::Stdout,
        tx,
    );

    Ok(Link {
        lines,
        input: Input::new(Some(Box::new(writer))),
        open,
    })
}

/// The reader the line pump sees.
///
/// A serial read that times out means "the board has not said anything yet",
/// not end-of-stream — a monitor that treated the two alike would stop at the
/// first idle second and look like a disconnection. Closing is the *only*
/// thing that ends the stream, and that is what the flag says.
struct Wire<R> {
    reader: R,
    open: Arc<AtomicBool>,
}

impl<R: Read> Read for Wire<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if !self.open.load(Ordering::Relaxed) {
                return Ok(0);
            }
            match self.reader.read(buffer) {
                // A zero-length read on a port with a timeout is the same
                // "nothing yet" as a timeout, and some drivers report it that
                // way instead. Returning it would end the stream.
                Ok(0) => continue,
                Ok(read) => return Ok(read),
                Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::Interrupted) => {
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers with whatever the script says, in order: `Err(TimedOut)` for an
    /// idle moment, bytes for a line.
    struct Script {
        steps: std::vec::IntoIter<std::io::Result<&'static [u8]>>,
    }

    impl Read for Script {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            match self.steps.next() {
                Some(Ok(bytes)) => {
                    buffer[..bytes.len()].copy_from_slice(bytes);
                    Ok(bytes.len())
                }
                Some(Err(e)) => Err(e),
                // Ran out of script: block forever, as an idle port does.
                None => Err(std::io::Error::from(ErrorKind::TimedOut)),
            }
        }
    }

    #[test]
    fn a_timeout_is_silence_not_end_of_stream() {
        // The bug this exists to prevent: a board that says nothing for a
        // second looks disconnected, and the plot stops for good.
        let open = Arc::new(AtomicBool::new(true));
        let mut wire = Wire {
            reader: Script {
                steps: vec![
                    Err(std::io::Error::from(ErrorKind::TimedOut)),
                    Ok(&b"[rusty:tel] gyro_x=1"[..]),
                    Err(std::io::Error::from(ErrorKind::TimedOut)),
                    Ok(&b"[rusty:tel] gyro_x=2"[..]),
                ]
                .into_iter(),
            },
            open: Arc::clone(&open),
        };

        let mut buffer = [0u8; 64];
        let read = wire.read(&mut buffer).unwrap();
        assert_eq!(&buffer[..read], b"[rusty:tel] gyro_x=1");
        let read = wire.read(&mut buffer).unwrap();
        assert_eq!(&buffer[..read], b"[rusty:tel] gyro_x=2");
    }

    #[test]
    fn closing_is_what_ends_the_stream() {
        let open = Arc::new(AtomicBool::new(true));
        let mut wire = Wire {
            reader: Script {
                steps: Vec::new().into_iter(),
            },
            open: Arc::clone(&open),
        };
        open.store(false, Ordering::Relaxed);
        assert_eq!(wire.read(&mut [0u8; 8]).unwrap(), 0);
    }
}
