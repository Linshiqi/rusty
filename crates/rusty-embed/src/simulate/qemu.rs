//! The emulator's command line beyond the boot itself — the pin channel,
//! the monitor, a free port for either — and one exchange with its machine
//! protocol.

/// Where pin changes leave the emulator and host-driven levels go back in.
///
/// Its own chardev, not the serial line: the UART belongs to the firmware,
/// and interleaving the two would make each unreadable to whoever wanted the
/// other. `-global` rather than `-device` because the machine creates the
/// GPIO device itself — there is no `-device` line to hang a chardev off.
///
/// QEMU listens and rusty connects, which is the arrangement the CI gate
/// proves; having rusty listen would be a second arrangement nothing has
/// booted.
///
/// **And QEMU waits for the connection (`wait=on`) before the guest runs.**
/// With `wait=off` the guest started at once and the channel caught up
/// when the emulator's main loop got round to it — measured on Windows at
/// four hundred milliseconds, by which time a sensor's firmware had asked
/// for its `WHO_AM_I`, found nothing declared on the bus, and given up,
/// and a blinky's first edge had gone unreported. The sheet's analog
/// values and bus devices have to be said before the firmware looks for
/// them, and only a guest that has not started yet is guaranteed not to
/// have looked. The channel retries until it is hung up, so a waiting
/// emulator is never left waiting on a caller that has stopped trying.
pub fn pins_args(port: u16) -> Vec<String> {
    vec![
        "-chardev".to_string(),
        format!("socket,id=pins,host=127.0.0.1,port={port},server=on,wait=on"),
        "-global".to_string(),
        "driver=esp32.gpio,property=pins,value=pins".to_string(),
    ]
}

/// Where [`free_port`] looks: below the range every system hands out as the
/// source port of an outgoing connection — 32768 up on Linux, 49152 up on
/// Windows and macOS.
const QUIET_PORTS: std::ops::Range<u16> = 20_000..32_000;

/// A port nothing else is on, learned by binding and letting go: where the
/// emulator listens for the pin channel, the monitor or the gdbstub.
///
/// QEMU listens and rusty connects, and the port is free between this
/// letting go and the emulator claiming it. **So it is not one the system
/// hands out by itself.** It was `bind(0)`'s, which draws from the same
/// range as every outgoing connection's source port, next to where that
/// range was handing out — so the next connection anywhere on the machine
/// was likely to take it. Measured with `netstat` at the moment an
/// emulator's bind failed (`os error 10048`): the port was the source of
/// another run's connection to its own emulator, three ports along. And a
/// connection made to such a port before it is listened on can be given it
/// as its own source, and meet itself ([`connect_local`]). Neither can
/// happen below the range. Each process starts its walk somewhere of its
/// own and each call steps on, so two runs do not probe the same port; a
/// port a server already holds fails the bind and the walk moves past it.
/// A fixed port would be a second simulation failing to start for a reason
/// the panel cannot explain.
pub fn free_port() -> Option<u16> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static STEP: AtomicU32 = AtomicU32::new(0);
    let span = u32::from(QUIET_PORTS.end - QUIET_PORTS.start);
    let start = std::process::id().wrapping_mul(2_654_435_761);
    for _ in 0..256 {
        // 7919 is prime and shares no factor with the span, so the walk
        // visits every port of it before it comes round again.
        let step = STEP.fetch_add(1, Ordering::Relaxed);
        let port = QUIET_PORTS.start + (start.wrapping_add(step.wrapping_mul(7919)) % span) as u16;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Some(port);
        }
    }
    // A machine with the whole range taken or reserved: the system's choice,
    // with the race above, rather than no simulation at all.
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    listener.local_addr().ok().map(|address| address.port())
}

/// Connect to `port` on this machine — and refuse a connection to nothing
/// but itself.
///
/// With nobody listening yet, a connection can be given the very port it
/// dials as its own source port, and TCP's simultaneous open then joins the
/// socket to itself: `connect` succeeds, and every read waits for an answer
/// that can only be its own echo — while the emulator, waiting for rusty
/// before it boots, waits for ever. A port from `bind(0)` was exactly such a
/// port, and a loop that connects every ten milliseconds while an emulator
/// starts is that many chances: measured as about one run in a thousand
/// under load, both ends `127.0.0.1:61652`. [`free_port`] no longer gives
/// one, but a machine's ephemeral range can be moved to reach the ports it
/// does give. Such a socket is dropped, which gives the port back to the
/// emulator, and the connection reads as refused, so the caller's loop
/// tries again.
pub fn connect_local(port: u16) -> std::io::Result<std::net::TcpStream> {
    let socket = std::net::TcpStream::connect(("127.0.0.1", port))?;
    if joined_to_itself(&socket)? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            format!("connected to itself on {port}; nobody is listening there yet"),
        ));
    }
    Ok(socket)
}

/// Whether a connected socket's two ends are one address: TCP's
/// simultaneous open, met by a socket alone.
fn joined_to_itself(socket: &std::net::TcpStream) -> std::io::Result<bool> {
    Ok(socket.local_addr()? == socket.peer_addr()?)
}

/// Where QEMU listens for its machine protocol, so a running simulation can
/// be stopped and started again.
///
/// A socket of its own rather than the monitor multiplexed onto the console:
/// `-serial mon:stdio` puts the monitor on the same stdin the firmware reads,
/// and a `stop` typed there would be a line the firmware might have wanted.
/// `wait=off` because the emulator must boot whether or not anybody is
/// listening — a simulation that waited for a debugger nobody attached is a
/// window that never fills.
pub fn qmp_args(port: u16) -> Vec<String> {
    vec![
        "-qmp".to_string(),
        format!("tcp:127.0.0.1:{port},server=on,wait=off"),
    ]
}

/// One exchange with a running emulator's machine protocol: the greeting,
/// the handshake, one command, its answer.
///
/// A connection per call. QMP wants `qmp_capabilities` before it takes
/// anything, so a held socket saves nothing and gives a run one more thing
/// to leak; and each answer is read before the next line goes out, so a
/// refusal is attributed to the command that caused it rather than to
/// whichever came after.
pub fn qmp(port: u16, verb: &str) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};

    let socket = connect_local(port)
        .map_err(|error| format!("the emulator's monitor did not answer on {port}: {error}"))?;
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok();
    let mut writer = socket
        .try_clone()
        .map_err(|error| format!("could not talk to the monitor: {error}"))?;
    let mut reader = BufReader::new(socket);

    let mut exchange = |request: Option<String>| -> Result<String, String> {
        if let Some(request) = request {
            writeln!(writer, "{request}")
                .map_err(|error| format!("could not write to the monitor: {error}"))?;
            writer
                .flush()
                .map_err(|error| format!("could not write to the monitor: {error}"))?;
        }
        loop {
            let mut line = String::new();
            let read = reader
                .read_line(&mut line)
                .map_err(|error| format!("the monitor stopped answering: {error}"))?;
            if read == 0 {
                return Err("the monitor closed while we were talking to it".to_string());
            }
            // Events arrive unasked; the answer is the line that is not one.
            if line.contains("\"event\"") {
                continue;
            }
            return Ok(line);
        }
    };

    exchange(None)?;
    exchange(Some("{\"execute\":\"qmp_capabilities\"}".to_string()))?;
    let answer = exchange(Some(format!("{{\"execute\":\"{verb}\"}}")))?;
    if answer.contains("\"error\"") {
        return Err(format!("the emulator refused to {verb}: {}", answer.trim()));
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The connection a loop can make while an emulator starts: a socket
    /// whose source port is the port it dials, nobody listening there. Made
    /// on purpose here — bound first, then connected to its own address —
    /// where under load it happened about one run in a thousand.
    #[test]
    fn a_socket_joined_to_itself_is_told_from_a_connection() {
        use socket2::{Domain, Socket, Type};

        let socket = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
        socket
            .bind(&std::net::SocketAddr::from(([127, 0, 0, 1], 0)).into())
            .unwrap();
        let own = socket.local_addr().unwrap();
        socket
            .connect(&own)
            .expect("TCP's simultaneous open, alone");
        let alone: std::net::TcpStream = socket.into();
        assert!(joined_to_itself(&alone).unwrap(), "{own:?}");

        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let real = connect_local(port).expect("somebody is listening");
        assert!(!joined_to_itself(&real).unwrap());
    }

    /// A port the emulator is to listen on comes from below the range the
    /// system takes source ports from, can be bound, and is not the one the
    /// last call gave.
    #[test]
    fn a_free_port_is_one_no_connection_is_given() {
        let first = free_port().expect("a port");
        let second = free_port().expect("a port");
        for port in [first, second] {
            assert!(QUIET_PORTS.contains(&port), "{port}");
            std::net::TcpListener::bind(("127.0.0.1", port)).expect("still free");
        }
        assert_ne!(first, second);
    }
}
