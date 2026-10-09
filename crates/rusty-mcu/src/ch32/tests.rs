use super::*;

/// ch32-hal's own PWM example, built for the J4M6 with nightly and
/// `-Zbuild-std=core` from `tests/fixtures/ch32v003-pwm` — its source and
/// lockfile are beside it. TIM1 channel 4 on PC4 at 1 kHz, the duty walked
/// up and down a hundredth every 10 ms, and SDI print saying where it is.
const PWM: &[u8] = include_bytes!("../../tests/fixtures/ch32v003-pwm/pwm.elf");
const PC4: u8 = 16 + 4;

/// The same example for the CH32X035F8U6, built with stable Rust for
/// `riscv32imc-unknown-none-elf` from `tests/fixtures/ch32x035-pwm`: TIM1
/// channel 4 on PB12 (remap 2) at 1 kHz, a button on PB1 holding it at
/// full, the duty a multiply and a divide away from the level.
const X035_PWM: &[u8] = include_bytes!("../../tests/fixtures/ch32x035-pwm/pwm.elf");
const PB12: u8 = 24 + 12;
const PB1: u8 = 24 + 1;

fn run(ms: u64) -> Vec<Event> {
    let mut machine = Machine::new(&CH32V003, PWM).expect("the fixture is a CH32V003 image");
    machine.run_until(ms * 1000);
    machine.take_events()
}

fn console(events: &[Event]) -> Vec<(u64, String)> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Console(line) => Some((event.at_us, line.clone())),
            _ => None,
        })
        .collect()
}

fn duties(events: &[Event], pin: u8) -> Vec<(u64, f64, f64)> {
    events
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Pwm { pin: p, duty, hz } if p == pin => Some((event.at_us, duty, hz)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_firmware_boots_through_ch32_hal_and_prints_over_sdi() {
    let events = run(50);
    let lines = console(&events);
    let first = lines.first().map(|(_, line)| line.as_str());
    assert!(
        first.is_some_and(|line| line.starts_with("pwm max duty ")),
        "the first line printed: {first:?}; every event: {events:#?}"
    );
    // Nothing the PWM example touches is left unmodelled, and nothing
    // faulted.
    let notes: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Note(note) => Some(note.as_str()),
            _ => None,
        })
        .collect();
    assert!(notes.is_empty(), "{notes:#?}");
}

#[test]
fn channel_four_drives_pc4_at_one_kilohertz_and_its_duty_climbs() {
    let events = run(600);
    let seen = duties(&events, PC4);
    assert!(seen.len() > 40, "a duty every 10 ms: {seen:?}");
    for (_, _, hz) in &seen {
        assert!((hz - 1000.0).abs() < 1.0, "1 kHz: {hz}");
    }
    // Up a hundredth every 10 ms, in time with the part's own clock.
    let at = |ms: u64| {
        seen.iter()
            .take_while(|(at_us, _, _)| *at_us <= ms * 1000)
            .last()
            .map(|(_, duty, _)| *duty)
            .unwrap_or(0.0)
    };
    assert!(at(100) < at(300) && at(300) < at(550), "{seen:?}");
    let expected = (at(550) - at(300)) / 0.25;
    assert!(
        (0.9..1.1).contains(&expected),
        "a quarter of the range in 250 ms: {}",
        at(550) - at(300)
    );
}

#[test]
fn the_ramp_turns_round_after_a_second() {
    let events = run(1_300);
    let lines = console(&events);
    let up = lines
        .iter()
        .find(|(_, line)| line == "up")
        .map(|(at, _)| *at);
    let up = up.unwrap_or_else(|| panic!("no `up` printed: {lines:?}"));
    // A hundred 10 ms steps, plus the setup before them.
    assert!((990_000..1_100_000).contains(&up), "`up` at {up} µs");
    let seen = duties(&events, PC4);
    let peak = seen.iter().map(|(_, duty, _)| *duty).fold(0.0, f64::max);
    assert!(peak > 0.95, "nearly full before it turns: {peak}");
    let last = seen.last().map(|(_, duty, _)| *duty).unwrap_or(1.0);
    assert!(last < peak - 0.1, "and falling after: {last} after {peak}");
}

#[test]
fn an_image_for_somewhere_else_is_refused_by_address() {
    let mut elf = PWM.to_vec();
    // Point the first program header's physical address at 0x9000_0000.
    let phoff = u32::from_le_bytes(elf[28..32].try_into().unwrap()) as usize;
    elf[phoff + 12..phoff + 16].copy_from_slice(&0x9000_0000u32.to_le_bytes());
    let refused = Machine::new(&CH32V003, &elf).err().expect("refused");
    assert!(refused.contains("0x90000000"), "{refused}");
}

/// A preloaded duty reaches the pin at the next update event, so every
/// report lands on the 1 kHz period's grid — and the steps are 10 ms of
/// SysTick delay plus the loop's own few dozen microseconds, so most are
/// 10 ms apart and one in every few dozen, where the drift crosses a period,
/// 11. The 20 ms gaps are the turn of the ramp, where the same duty is
/// written twice and nothing changes.
#[test]
fn a_duty_lands_on_the_update_event_after_it_is_written() {
    let events = run(2_500);
    let seen = duties(&events, PC4);
    let first = seen[1].0;
    for (at, _, _) in &seen[1..] {
        assert_eq!(
            (at - first) % 1_000,
            0,
            "a report off the period's grid at {at}"
        );
    }
    let gaps: Vec<u64> = seen[1..]
        .windows(2)
        .map(|pair| pair[1].0 - pair[0].0)
        .collect();
    assert!(gaps.len() > 200, "{} steps", gaps.len());
    let tens = gaps.iter().filter(|gap| **gap == 10_000).count();
    let elevens = gaps.iter().filter(|gap| **gap == 11_000).count();
    let turns = gaps
        .iter()
        .filter(|gap| matches!(**gap, 20_000 | 21_000))
        .count();
    assert_eq!(tens + elevens + turns, gaps.len(), "{gaps:?}");
    assert!(
        tens > 10 * elevens,
        "{tens} steps of 10 ms, {elevens} of 11"
    );
}

fn notes(events: &[Event]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Note(note) => Some(note.as_str()),
            _ => None,
        })
        .collect()
}

/// A V4C: thirty-two registers, the M extension, a 64-bit SysTick, SDI
/// print at its own address, GINTENR — all of it on the way to the first
/// line, which is why that line is the test.
#[test]
fn the_x035_boots_through_ch32_hal_and_prints_over_sdi() {
    let mut machine = Machine::new(&CH32X035, X035_PWM).expect("the fixture is a CH32X035 image");
    machine.run_until(50_000);
    let events = machine.take_events();
    let lines = console(&events);
    assert_eq!(
        lines.first().map(|(_, line)| line.as_str()),
        Some("pwm max duty 8000"),
        "every event: {events:#?}"
    );
    assert!(notes(&events).is_empty(), "{:#?}", notes(&events));
}

/// 8 MHz out of reset, TIM1 counting to 8000 for 1 kHz; the duty walks up
/// a hundredth every 10 ms on PB12, which TIM1_RM = 2 put there.
#[test]
fn the_x035_drives_pb12_at_one_kilohertz_and_its_duty_climbs() {
    let mut machine = Machine::new(&CH32X035, X035_PWM).unwrap();
    machine.run_until(600_000);
    let events = machine.take_events();
    let seen = duties(&events, PB12);
    assert!(seen.len() > 40, "a duty every 10 ms: {seen:?}");
    assert!(
        seen.iter().all(|(_, _, hz)| (hz - 1000.0).abs() < 1.0),
        "{seen:?}"
    );
    let at = |ms: u64| {
        seen.iter()
            .take_while(|(at_us, _, _)| *at_us <= ms * 1000)
            .last()
            .map(|(_, duty, _)| *duty)
            .unwrap_or(0.0)
    };
    assert!(at(100) < at(300) && at(300) < at(550), "{seen:?}");
    let quarter = at(550) - at(300);
    assert!(
        (0.22..0.28).contains(&quarter),
        "a quarter in 250 ms: {quarter}"
    );
    // What the firmware printed is what the arithmetic says: duty is
    // 8000 * level / 100, at every quarter.
    let lines = console(&events);
    assert!(
        lines.iter().any(|(_, line)| line == "level 25 duty 2000"),
        "{lines:?}"
    );
}

/// The button pulls PB1 low through the pull-up the firmware set, and the
/// duty goes to full while it is held.
#[test]
fn holding_the_x035_button_holds_the_duty_at_full() {
    let mut machine = Machine::new(&CH32X035, X035_PWM).unwrap();
    machine.run_until(100_000);
    machine.take_events();
    machine.drive(PB1, Some(false));
    machine.run_until(200_000);
    let held = duties(&machine.take_events(), PB12);
    assert!(
        held.last().is_some_and(|(_, duty, _)| *duty > 0.999),
        "full while held: {held:?}"
    );
    machine.drive(PB1, None);
    machine.run_until(300_000);
    let released = duties(&machine.take_events(), PB12);
    assert!(
        released.last().is_some_and(|(_, duty, _)| *duty < 0.5),
        "the ramp again once let go: {released:?}"
    );
}

/// A V003 image is linked at the same address as an X035 one, so the
/// address cannot refuse it — but its first instruction can: it names a
/// register the V2A has and is built for a target the V4C would also
/// run. What tells them apart is the flash's size, and an image that
/// needs more than the part has is refused by it.
#[test]
fn an_image_bigger_than_the_parts_flash_is_refused_by_its_size() {
    let mut elf = X035_PWM.to_vec();
    let phoff = u32::from_le_bytes(elf[28..32].try_into().unwrap()) as usize;
    // Load the first segment 60 KB in: past the V003's 16 KB.
    elf[phoff + 12..phoff + 16].copy_from_slice(&0x0800_F000u32.to_le_bytes());
    let refused = Machine::new(&CH32V003, &elf).err().expect("refused");
    assert!(refused.contains("CH32V003"), "{refused}");
}
