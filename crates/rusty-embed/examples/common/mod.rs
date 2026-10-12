//! What the probes that sit on the emulator's pin channel share. A
//! directory rather than `examples/common.rs`, which cargo would take for
//! an example of its own.

use std::net::TcpStream;
use std::time::Duration;

/// The pin channel's socket, retried for ten seconds while the emulator
/// comes up to listen on it.
pub fn connect(port: u16) -> Option<TcpStream> {
    for _ in 0..100 {
        if let Ok(socket) = TcpStream::connect(("127.0.0.1", port)) {
            return Some(socket);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}
