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

/// A port nothing else is on, learned by binding and letting go: where the
/// emulator listens for the pin channel, the monitor or the gdbstub.
///
/// QEMU listens and rusty connects. The gap between releasing this and QEMU
/// claiming it is a race in theory; in practice the alternative is a fixed
/// port, and a fixed port is a second simulation failing to start for a
/// reason the panel cannot explain.
pub fn free_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
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

    let socket = std::net::TcpStream::connect(("127.0.0.1", port))
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
