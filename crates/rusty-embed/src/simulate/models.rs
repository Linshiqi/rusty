//! Which peripherals an emulator binary models, read off the binary itself:
//! a marker only rusty's build emits for each, so a stock build dropped in
//! the same directory gets the right answer.

use std::path::Path;

/// The line only rusty's GPIO model emits, and the thing to look for in a
/// binary to know whether it has one.
///
/// A marker in the emulator's own output rather than a version file beside
/// it: the question is "does *this* binary keep pin state", and a user who
/// dropped Espressif's build into the same directory must get the right
/// answer. The CI gate greps the same literal for the same reason.
pub(super) const GPIO_MODEL_MARKER: &[u8] = b"[rusty:gpio@";
/// And the converter's, which is a *later* build than the GPIO model's.
///
/// The two are asked separately because they arrived separately: a build
/// from before `qemu-v3` models the pins and has no SAR converter at all,
/// and there is one such copy in the data directory of every machine that
/// installed rusty early. Asking `has_gpio_model` and reading that as "the
/// emulator has the peripherals" is the proxy-check mistake `find_gdb`
/// already taught: the run gets as far as the firmware's own
/// `adc.read_oneshot()` and hangs there, and the failure names a
/// conversion rather than an emulator.
const ADC_MODEL_MARKER: &[u8] = b"[rusty:adc@";
/// And the two buses', which arrived with the converter's generation and
/// are asked for by name all the same: an assumption that one marker stands
/// for three models is the proxy check again.
const I2C_MODEL_MARKER: &[u8] = b"[rusty:i2c@";
const SPI_MODEL_MARKER: &[u8] = b"[rusty:spi@";
/// And the duty timer's and the strip channel's, which arrived a
/// generation later. **Every model this version drives is asked for by
/// name**, one marker each: a build that has four of the six answers every
/// question about the four and hangs the firmware inside `wait()` or
/// leaves a servo still, which is the failure an "it has the peripherals"
/// proxy would wave through.
const PWM_MODEL_MARKER: &[u8] = b"[rusty:pwm@";
const RMT_MODEL_MARKER: &[u8] = b"[rusty:rmt@";
/// And the pads themselves: a build that can *join* two of them, and that
/// answers an input nobody is driving with the pad's own pull. One marker
/// for the two because they are one replacement file built together —
/// halves of the same model rather than a proxy for each other. Without
/// them a keypad's key reaches an emulator that drops the line, and every
/// `Pull::Up` button reads as held down from reset.
const PAD_MODEL_MARKER: &[u8] = b"[rusty:sw@";
/// Every model this rusty drives on both machines, in one list: what
/// `has_peripherals` requires and what ranks one copy of the emulator
/// against another.
const PERIPHERAL_MARKERS: [&[u8]; 6] = [
    ADC_MODEL_MARKER,
    I2C_MODEL_MARKER,
    SPI_MODEL_MARKER,
    PWM_MODEL_MARKER,
    RMT_MODEL_MARKER,
    PAD_MODEL_MARKER,
];

/// And the ESP32's own: every interrupt source reaching its handler — the
/// interrupt matrix keeping each source's level, answering the status words
/// the dispatcher reads and driving a CPU line from every source mapped to
/// it, and the timer group raising a level interrupt the way this part
/// enables one — on top of the peripherals in this part's layout and the
/// FPU on from reset, which arrived a generation earlier. There is no line
/// on the channel to recognise it by, so the marker is the name of the
/// matrix's status region, which only this generation declares.
///
/// **Asked of the Xtensa binary alone** (`markers_of`), because the matrix
/// is compiled into no other: asked of the RISC-V one it would call every
/// current C3 build out of date. A copy without it runs an ESP32 whose
/// timer never interrupts, so an Embassy application's `Timer::after()`
/// never returns. The one before it answered the status words through a
/// region of rusty's own device, `esp32.gpio.intr-status`, for GPIO's
/// source alone — which is why a marker names what a build does rather
/// than which build it is.
const ESP32_MODEL_MARKER: &[u8] = b"misc.esp32.intmatrix.status";

/// Tables played against the virtual clock: a signal on a pin or on a
/// sensor's register block, sample-exact in the firmware's own time
/// (`docs/signals.md`). A capability of its own rather than one of
/// `PERIPHERAL_MARKERS`: a build without it runs every firmware exactly as
/// before, and only a sheet with a signal on it needs the upgrade — counted
/// there, it would call every earlier ESP32 build out of date under a limit
/// that talks about interrupts.
const WAVE_MODEL_MARKER: &[u8] = b"[rusty:wave@";

/// A systimer that keeps the virtual clock's time. Upstream's counter
/// dropped the fraction of a tick at every read, so firmware polling it ran
/// slow by however often it looked — about half a percent on a runner, a
/// few on a slow machine — and a signal played against the virtual clock
/// reached it at another frequency by its own. Nothing else of the fix is
/// left in the binary to recognise it by, so `qemu/patches.py` leaves this.
/// Part of playing a signal rather than a capability of its own: a table
/// is only in the firmware's time when the firmware's clock keeps that
/// time, so a build with the tables and without this is outdated for a
/// sheet that plays anything.
const CLOCK_MODEL_MARKER: &[u8] = b"[rusty:systimer-exact]";

/// The markers this binary has to carry: every machine's, and the ESP32's
/// when it is the machine that runs one.
pub(super) fn markers_of(qemu: &Path) -> impl Iterator<Item = &'static [u8]> {
    let xtensa = qemu
        .file_stem()
        .is_some_and(|stem| stem == "qemu-system-xtensa");
    PERIPHERAL_MARKERS
        .into_iter()
        .chain(xtensa.then_some(ESP32_MODEL_MARKER))
}

/// Does this emulator model the converter, both buses, LEDC, RMT and the
/// pads' own pulls and switches — and, for an ESP32, its interrupts?
pub fn has_peripherals(qemu: &Path) -> bool {
    markers_of(qemu).all(|marker| carries(qemu, marker))
}

/// How many of rusty's models this binary carries, pins included — the
/// number one copy is ranked against another by.
pub(super) fn models_carried(qemu: &Path) -> usize {
    usize::from(has_gpio_model(qemu))
        + usize::from(has_wave_model(qemu))
        + markers_of(qemu)
            .filter(|marker| carries(qemu, marker))
            .count()
}

/// Does this emulator play a signal against the firmware's own clock — the
/// tables, and a firmware clock that keeps the time they are played in?
pub fn has_wave_model(qemu: &Path) -> bool {
    carries(qemu, WAVE_MODEL_MARKER) && carries(qemu, CLOCK_MODEL_MARKER)
}

/// Does this emulator model GPIO, or is it the stock one whose write handler
/// is an empty function?
///
/// Everything downstream branches on this: with the model, the board shows
/// what a pin *is* and a button drives the register the firmware reads;
/// without it, the firmware has to narrate its own pins over the serial line
/// and a button arrives as a `B14=1` message. Both work — but claiming the
/// first while running the second would show a dark LED for correct firmware,
/// which is the failure this whole path exists to remove.
///
/// Cached on path, length and mtime, because it is asked once per run and the
/// answer costs a scan of a hundred-megabyte file.
pub fn has_adc_model(qemu: &Path) -> bool {
    carries(qemu, ADC_MODEL_MARKER)
}

pub fn has_gpio_model(qemu: &Path) -> bool {
    carries(qemu, GPIO_MODEL_MARKER)
}

/// Does this binary carry `marker`, asked once per binary?
///
/// Cached on path, length and mtime, because each is asked at every plan and
/// every run, and the answer costs a scan of a file tens of megabytes long.
pub(super) fn carries(qemu: &Path, marker: &'static [u8]) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    type Stamp = (
        std::path::PathBuf,
        u64,
        Option<std::time::SystemTime>,
        &'static [u8],
    );
    static SEEN: OnceLock<Mutex<HashMap<Stamp, bool>>> = OnceLock::new();

    let Ok(meta) = std::fs::metadata(qemu) else {
        return false;
    };
    let stamp: Stamp = (qemu.to_path_buf(), meta.len(), meta.modified().ok(), marker);

    let cache = SEEN.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(seen) = cache.lock()
        && let Some(known) = seen.get(&stamp)
    {
        return *known;
    }

    let found = scan_for(qemu, marker);
    if let Ok(mut seen) = cache.lock() {
        seen.insert(stamp, found);
    }
    found
}

/// Is `needle` anywhere in this file?
///
/// Chunked with an overlap of `needle.len() - 1`, so a marker straddling a
/// chunk boundary is still found — reading a hundred megabytes into memory to
/// avoid thinking about that is the version of this that makes the toolchain
/// panel stutter.
pub(super) fn scan_for(path: &Path, needle: &[u8]) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let overlap = needle.len() - 1;
    let mut buffer = vec![0u8; 1 << 20];
    let mut filled = 0usize;
    loop {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => return false,
            Ok(read) => {
                filled += read;
                if buffer[..filled].windows(needle.len()).any(|w| w == needle) {
                    return true;
                }
                if filled + overlap >= buffer.len() {
                    buffer.copy_within(filled - overlap..filled, 0);
                    filled = overlap;
                }
            }
            Err(_) => return false,
        }
    }
}
